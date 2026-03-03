# MLS Protocol Usage

## Overview

Conclave uses the Messaging Layer Security (MLS) protocol ([RFC 9420](https://www.rfc-editor.org/rfc/rfc9420.txt)) for all end-to-end encryption. Every conversation — whether between two people or a large group — is an MLS group. There is no separate direct messaging protocol. All cryptographic operations (key generation, encryption, decryption, group state management) are performed exclusively on the client.

## Service Model

RFC 9420 Section 3 defines two external services that every MLS deployment requires:

- **Authentication Service (AS)**: Authenticates the credentials presented by group members, providing the trusted binding between human identities and MLS signing keys.
- **Delivery Service (DS)**: Routes MLS messages among participants. The DS is largely untrusted — MLS guarantees confidentiality and integrity regardless of DS behavior.

In Conclave, a single server provides both services:

**Delivery Service**: The server stores and forwards opaque MLS messages (application ciphertexts, commits, welcomes, key packages, GroupInfo) without interpreting their contents. It broadcasts real-time event notifications to connected clients via SSE.

**Authentication Service**: The server maintains the authoritative identity registry, binding usernames to user IDs to MLS signing key fingerprints. All MLS operations require bearer-token authentication, ensuring only the registered owner of a `user_id` can publish credentials for that identity. Clients complement the server-side AS with [TOFU fingerprint verification](tofu.md) to detect post-first-contact key changes.

Unlike some MLS deployments that separate the AS and DS into distinct services, Conclave combines them in a single process for deployment simplicity. This means a compromised server can manipulate both identity bindings (AS) and message routing (DS). See [Architecture Overview](../architecture/overview.md#service-roles) for the trust model and [Server Compromise](../security/threats.md#threat-server-compromise) for the threat analysis.

## Cipher Suite

Conclave uses MLS cipher suite **CURVE448_CHACHA** (cipher suite ID 6 as defined in RFC 9420 Section 17.1):

| Component | Algorithm |
|-----------|-----------|
| Key Exchange (KEM) | X448 |
| Authenticated Encryption (AEAD) | ChaCha20-Poly1305 |
| Hash | SHA-512 |
| Signature | Ed448 |
| Security Level | 256-bit |

All clients and servers in a Conclave deployment MUST support this cipher suite. All groups MUST use this cipher suite.

## Sync Mode

MLS operations are CPU-bound cryptographic computations. The MLS library runs in **synchronous mode** on the client. Client implementations that use asynchronous runtimes (e.g., tokio, async-std) SHOULD offload MLS operations to a blocking task pool to avoid stalling the event loop.

## Epoch Retention

MLS groups advance through epochs on each commit (member add, member remove, key rotation, external rejoin). Clients need to retain key material from prior epochs to decrypt messages that were encrypted under those epochs.

Clients SHOULD retain key material for at least **16 prior epochs**. This allows a client to be offline through up to 16 group state transitions (commits) and still decrypt messages sent during those epochs.

Regular application messages (chat) do not advance the epoch. A client can be offline through an unlimited number of chat messages within the same epoch.

## Decryption Error Handling

When a client fails to decrypt a message (e.g., because the epoch's key material has been evicted), the client SHOULD:

1. Display a warning to the user indicating the message could not be decrypted, including the message's sequence number and the reason for failure.
2. **Advance the sequence tracking** past the undecryptable message. Failed messages cannot be retried — blocking on them would cause infinite retry loops.
3. Continue processing subsequent messages.

If a user experiences persistent decryption failures, they can perform an [account reset](../flows/account-reset.md) to rejoin the group with fresh cryptographic state.

## MLS Operations Summary

The following MLS operations are used in Conclave:

| Operation | MLS Primitive | Conclave Usage |
|-----------|--------------|----------------|
| Generate signing identity | `signature_key_generate()` | Registration, account reset |
| Generate key package | `generate_key_package_message()` | Pre-publishing credentials |
| Create group | `create_group()` + `commit_builder().build()` | New group creation |
| Invite member | `commit_builder().add_member(key_package).build()` | Adding members to groups |
| Join group | `join_group(welcome_message)` | Processing a Welcome after invite acceptance |
| Encrypt message | `encrypt_application_message(plaintext)` | Sending chat messages |
| Decrypt message | `process_incoming_message(ciphertext)` | Receiving chat messages and commits |
| Remove member | `commit_builder().remove_member(index).build()` | Kicking a member |
| Leave group | `commit_builder().remove_member(own_index).build()` | Voluntary departure |
| Rotate keys | `commit_builder().build()` (empty commit) | Forward secrecy, phantom leaf cleanup |
| External rejoin | `external_commit_builder().build(group_info)` | Account reset rejoin |
| Export GroupInfo | `group_info_message_allowing_ext_commit(true)` | Enabling external commits |
