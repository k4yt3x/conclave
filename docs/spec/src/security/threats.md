# Threat Model and Mitigations

## Threat: Username Enumeration

**Attack**: An attacker probes the login endpoint with different usernames to determine which accounts exist, by measuring response time differences.

**Mitigation**: The login endpoint performs timing-equalized password verification. When the requested username does not exist, the server runs `verify_password()` against a precomputed dummy Argon2id hash, ensuring both code paths (valid user, invalid user) have equivalent computational profiles.

## Threat: Key Package Exhaustion

**Attack**: An attacker rapidly consumes a target user's regular key packages, forcing fallback to the reusable last-resort package. The last-resort package carries security risks per RFC 9420 Section 16.8 (e.g., replay of group additions).

**Mitigation**:

- The `GET /api/v1/key-packages/{user_id}` endpoint is **rate-limited to 10 requests per minute per target user**.
- Last-resort key packages are **never deleted** — they ensure the user is always reachable even if all regular packages are exhausted.
- Clients automatically replenish key packages after consumption.

## Threat: MLS Wire Format Injection

**Attack**: An attacker uploads malformed or non-MLS data as key packages, which could cause failures when other users attempt to use those packages for group operations.

**Mitigation**: The server validates all uploaded key packages for MLS wire format correctness:

- MLS version MUST be `0x0001` (MLS 1.0).
- Wire format type MUST be `0x0005` (`mls_key_package`).
- Size MUST be between 4 bytes and 16 KiB.

This prevents obviously malformed data from being stored. Note that the server does NOT perform full cryptographic validation — that responsibility falls on the consuming client.

## Threat: Registration Abuse

**Attack**: An attacker creates many accounts to spam users or exhaust server resources.

**Mitigation**:

- Server operators can disable public registration (`registration_enabled: false`).
- When registration is disabled, a valid registration token is required. Token comparison uses constant-time equality to prevent timing-based guessing.
- The registration token format is restricted to `[a-zA-Z0-9_-]`, validated at config load time.

## Threat: Invite Spam

**Attack**: A malicious admin adds users to groups without their consent, flooding their client with unwanted group memberships.

**Mitigation**: The two-phase [escrow invite system](../flows/escrow-invite.md) requires explicit acceptance:

1. The admin can only upload an invite to escrow — the target is not added to the group.
2. The target receives a notification and can inspect the invitation.
3. The target must explicitly accept to join, or decline to discard the invite.
4. The server enforces a unique constraint on `(group_id, invitee_id)` — only one pending invite per user per group.
5. Pending invites have a configurable TTL (default: 7 days) and are cleaned up by the server's background task.

## Threat: Unauthorized Group Access

**Attack**: A user who was never a group member attempts to join via external commit.

**Mitigation**: The external join endpoint requires:

- The user MUST be an existing server-side member of the group.
- A stored MLS GroupInfo MUST exist, and GroupInfo is only set by authorized members through commit uploads, member removals, or leave operations.

## Threat: Key Substitution Attack

**Attack**: A compromised server substitutes a user's signing public key with one controlled by the attacker, allowing the attacker to impersonate the user.

**Mitigation** (partial):

- The [TOFU fingerprint verification](../mls/tofu.md) system stores the first-seen fingerprint for each user. Any subsequent change triggers a `[!]` warning.
- Users can perform out-of-band fingerprint verification via `/verify` to confirm fingerprints through a trusted channel, eliminating the first-contact vulnerability.

**Limitation**: TOFU does not protect against first-contact attacks. If the server is compromised during the initial key exchange, it could substitute a different key before the client stores the fingerprint.

## Threat: Message Replay

**Attack**: An attacker replays previously observed MLS ciphertexts to inject duplicate messages.

**Mitigation**: MLS provides built-in replay protection:

- Each epoch has unique key material.
- Per-message keys are derived from the epoch's key schedule and consumed on use.
- The MLS `process_incoming_message()` function rejects replayed messages.

Additionally, the server assigns monotonically increasing sequence numbers to messages. Clients track their last-seen sequence number and only fetch messages with higher sequence numbers.

## Threat: Server Compromise

If an attacker compromises the server, they can:

- **NOT** read message contents — all messages are E2E encrypted MLS ciphertexts.
- **NOT** forge messages — they lack users' MLS signing keys.
- **Observe metadata**: Who communicates with whom, group membership, message timing and frequency, IP addresses.
- **Substitute keys for new contacts**: Perform first-contact key substitution before TOFU fingerprints are stored (mitigated by out-of-band `/verify`).
- **Block messages**: Prevent message delivery or selectively drop events.
- **Disrupt service**: Delete groups, remove members, or take the server offline.

### What Server Compromise Does NOT Allow

The fundamental security guarantee is that message content remains confidential even if the server is fully compromised, because the server never has access to MLS key material. The server is an untrusted relay by design.

## Trust Model Limitations

Conclave currently uses `BasicCredential` with `BasicIdentityProvider`, which does NOT validate that MLS credentials correspond to legitimate users. Trust in user identities relies on:

1. **Server authentication gate**: Only authenticated users can upload key packages.
2. **TOFU fingerprint tracking**: Detects key changes after first contact.
3. **Out-of-band verification**: `/verify` command for strong fingerprint confirmation.

For communities requiring stronger identity assurance (e.g., binding MLS identities to organizational PKI), future versions may support X.509 credentials with certificate authority validation.

## Threat: Active Probing / Server Fingerprinting

**Attack**: An adversary sends probe requests to discover whether a server is running a messaging service. Distinctive API paths, content types, or response patterns reveal the service's identity, allowing the adversary to block or target the server.

**Mitigations**:

- Deploy behind an authenticating reverse proxy so unauthenticated probes never reach the application.
- Serve the application under a non-default path prefix; expose a benign site at the default path.
- Use a CDN or tunnel service to obscure the origin server's identity and IP address.

See [Censorship Circumvention](/censorship.html) in the user guide for deployment guidance.

## Threat: Origin Server Discovery

**Attack**: An adversary identifies the origin server's IP address through DNS records, certificate transparency logs, or direct connections, bypassing CDN or proxy protections.

**Mitigations**:

- Restrict the origin server's firewall to only accept connections from the CDN or tunnel service.
- Use tunnel-based exposure so the origin server has no public IP or open inbound ports.
- Avoid leaking the origin IP in TLS certificates, DNS records, or error pages.

## Threat: Traffic Analysis

**Attack**: An adversary observes connection timing, message volume, and traffic patterns to infer messaging activity, even without access to message contents.

**Limitation**: The countermeasures above do not address traffic analysis. Message timing, frequency, and connection patterns remain observable at the network level. Dedicated traffic-shaping or padding mechanisms would be needed but are outside the current scope.
