# Security Properties

## End-to-End Encryption

All application messages in Conclave are MLS `PrivateMessage` ciphertexts. The server stores and forwards opaque encrypted blobs without any ability to access the plaintext content.

The server MUST NOT attempt to decrypt, interpret, or validate message contents. Implementations MUST ensure that no plaintext message data is logged, cached, or otherwise persisted on the server.

## Forward Secrecy

MLS provides forward secrecy through key ratcheting. Each epoch has distinct key material, and advancing the epoch discards old keys. This means:

- Compromising a user's current keys does NOT reveal past messages.
- [Key rotation](../flows/key-rotation.md) (empty commits) explicitly advances the epoch to discard old key material.
- Regular application messages within an epoch use per-message keys derived from the epoch's key schedule.

## Post-Compromise Security

MLS commit operations rotate group key material, providing post-compromise security:

- After a member's keys are compromised, a subsequent commit operation (member add/remove, key rotation) generates new key material that the attacker cannot derive.
- The group recovers security as soon as any non-compromised member performs a commit.

## Authentication Security

### Password Storage

Passwords MUST be hashed using **Argon2id** with random salts before storage. Argon2id provides resistance against both GPU-based and side-channel attacks.

### Session Tokens

- Tokens MUST be 256-bit cryptographically random values (hex-encoded, 64 characters).
- Tokens MUST be generated using a cryptographically secure random number generator (e.g., OS entropy source).
- The server SHOULD store a hash (SHA-256) of the token rather than the raw token value.
- Tokens MUST have a configurable expiry (default: 7 days).
- Tokens MUST be revocable via the logout endpoint.

### Timing Attack Mitigation

The login endpoint MUST perform timing-equalized password verification:

- When the requested username exists: verify the provided password against the stored hash.
- When the requested username does NOT exist: verify the provided password against a precomputed dummy Argon2id hash.

This ensures both code paths have equivalent computational profiles, preventing username enumeration via timing analysis.

### Registration Token Security

When registration is token-gated, the registration token comparison MUST use constant-time equality (e.g., `subtle::ConstantTimeEq` or equivalent) to prevent timing-based token guessing.

## Transport Security

### TLS

The server supports two transport modes:

1. **Native TLS**: Direct HTTPS with server-terminated TLS.
2. **Plain HTTP**: For deployments behind a TLS-terminating reverse proxy.

Clients MUST validate the server's TLS certificate when connecting over HTTPS. Implementations MAY provide an option to accept invalid certificates for development/testing purposes, but this option MUST NOT be enabled in production deployments.

### HTTP/2

All communication uses HTTP/2, which provides:

- Multiplexed streams over a single TCP connection.
- Header compression.
- Binary framing.

## MLS Identity Key Protection

The MLS signing key pair is the most sensitive credential in the system. Compromise of the signing secret key allows an attacker to:

- Impersonate the user in MLS operations.
- Sign commits and messages as the user.
- Generate key packages on behalf of the user.

Clients MUST protect the signing secret key with appropriate filesystem permissions. The key SHOULD be stored with permissions restricting access to the owning user only (e.g., `0600` on Unix systems).

## Transactional Integrity

Critical server operations that modify multiple database tables MUST be performed atomically within a single database transaction:

- **Invite acceptance**: Deleting the pending invite, adding the member, storing the welcome, and storing the commit MUST all succeed or all fail.
- **Commit upload**: Storing the commit message and updating the GroupInfo MUST be atomic.

SSE notifications MUST be sent only after the transaction commits, ensuring clients never receive notifications for partially applied state changes.

## Invite Consent

All post-creation member additions require the target's explicit acceptance. There is no way to add a user to a group without their consent:

1. The admin pre-builds the MLS materials and uploads them to escrow.
2. The target receives a notification and can inspect the invitation.
3. The target explicitly accepts or declines.
4. Only on acceptance is the target added to the group.

This prevents invite spam and respects user autonomy.

## Admin Role Authorization

Group management operations (invite, remove, update group settings, promote, demote, cancel invite) require the `admin` role.

- The group creator is automatically assigned the `admin` role.
- New members receive the `member` role by default.
- Admins can promote members and demote other admins.
- The server MUST reject demotion of the last remaining admin, ensuring every group always has at least one admin.

## External Join Authorization

The `POST /api/v1/groups/{id}/external-join` endpoint requires:

- The user MUST already be a server-side member of the group.
- A stored MLS GroupInfo MUST exist for the group.

Since GroupInfo is only stored by authorized group members via commit upload, member removal, or leave operations, this prevents unauthorized users from joining arbitrary groups via external commit.
