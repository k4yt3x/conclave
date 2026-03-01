# Key Packages

## Purpose

Key packages are pre-published MLS credentials that enable asynchronous group additions. When user A wants to add user B to a group, A fetches one of B's key packages from the server and uses it to build an MLS Add proposal and Welcome message — even if B is offline. This is a core MLS concept defined in RFC 9420 Section 10.

## Lifecycle

### Initial Upload

After registration or login, a client SHOULD upload an initial set of key packages to the server:

- **5 regular key packages**: Single-use credentials consumed when the user is added to a group.
- **1 last-resort key package**: A reusable fallback that is never consumed.

Key packages are uploaded via `POST /api/v1/key-packages` with the `entries` field containing a list of `KeyPackageEntry` messages, each specifying the key package `data` and an `is_last_resort` flag.

### Consumption

When a user invites another user to a group, the server returns one of the target's key packages via the invite endpoint. Key packages are consumed in **FIFO order** (oldest regular package first).

- **Regular key packages** are deleted from the server after consumption.
- **Last-resort key packages** are returned but **never deleted**. Per RFC 9420 Section 16.6, the last-resort package ensures a user is always reachable even when all regular packages have been exhausted.

### Replenishment

After a client is added to a group (i.e., processes an MLS Welcome message), it SHOULD upload replacement key packages to maintain availability. The recommended practice is to upload one new regular key package for each Welcome processed.

### Server Limits

The server MUST enforce a maximum of **10 regular key packages** per user. Uploads beyond this cap SHOULD replace the oldest existing packages.

Uploading a new last-resort key package replaces the previous last-resort package for that user.

## Wire Format Validation

The server MUST validate all uploaded key packages for MLS wire format correctness per RFC 9420 Section 6:

- The first 2 bytes MUST be the MLS version (`0x0001` for MLS 1.0).
- The next 2 bytes MUST be the wire format type (`0x0005` for `mls_key_package`).
- The minimum size is 4 bytes.
- The maximum size is **16 KiB** (16,384 bytes).

The server does NOT perform full cryptographic validation of key package contents (signature verification, credential validation, etc.). This validation is the responsibility of the client that consumes the key package during group operations.

## Signing Key Fingerprint

When uploading key packages, the client SHOULD include a `signing_key_fingerprint` in the request. This is the SHA-256 hash of the client's MLS signing public key, represented as a 64-character lowercase hexadecimal string. The server stores this fingerprint and distributes it to other users for [TOFU identity verification](tofu.md).

## Rate Limiting

The key package consumption endpoint (`GET /api/v1/key-packages/{user_id}`) MUST be rate-limited to **10 requests per minute per target user**. This prevents an attacker from draining a user's regular key packages, which would force fallback to the reusable last-resort package (with associated reuse risks per RFC 9420 Section 16.8).
