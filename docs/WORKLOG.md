# Conclave Work Log

## 2026-02-15: Initial Implementation

### What Was Built

Complete server and client implementation from scratch:

- **conclave-proto**: Protobuf schema with 20+ message types, prost code generation.
- **conclave-server**: Full axum HTTP server with SQLite storage, Argon2id auth, opaque token sessions, 11 API endpoints, SSE real-time push.
- **conclave-client**: CLI client with MLS E2EE (mls-rs + OpenSSL), SQLite-persisted MLS state, one-shot commands, and interactive REPL.

End-to-end encryption verified: two users can register, create an MLS group, exchange encrypted messages, and decrypt them.

### Known Issues and Limitations

#### MLS Epoch Handling

When a user fetches messages, the list includes MLS commit messages (e.g., the initial commit from group creation at seq 1). For users who joined via a welcome message, these commits are for an epoch they've already processed. The client currently handles this by catching decryption errors and returning `None` (silently skipping). A cleaner solution would be to either:

1. Tag messages in the database with a type field (commit vs. application) so clients can skip commits they don't need.
2. Store commits and application messages in separate tables/endpoints.
3. Track each client's last processed sequence number server-side and only return newer messages.

#### Own Messages Not Visible

Due to MLS's ratcheting, a sender cannot decrypt their own messages from the server. The encrypted ciphertext was produced with a key that has since been ratcheted forward. In a production client, sent messages should be stored locally in plaintext alongside their sequence number so they can be displayed in the message history. The current implementation does not do this.

#### Single Key Package

Each user has at most one key package on the server at a time. Uploading a new one replaces the old one, and fetching one consumes it. This means a user can only be added to one group before needing to upload a new key package. A production system should support multiple key packages per user (upload N, consume one at a time).

#### No Member Removal

The `commit_builder().remove_member()` API exists in mls-rs but is not wired up in the client or server. There is no endpoint for removing members from a group.

#### No Message Ordering Guarantees

The server assigns sequence numbers per group, but there is no mechanism to ensure clients process messages in order. Out-of-order MLS message processing is enabled via the `rfc_compliant` feature (which includes `out_of_order`), but the client fetches all messages from seq 0 each time, which may cause issues if messages are processed multiple times. A cursor/watermark per user per group would fix this.

#### BasicCredential Trust Model

The MLS identity uses `BasicCredential` with `BasicIdentityProvider`, which accepts any credential without validation. This is fine for a closed community where the server is trusted to enforce usernames, but provides no cryptographic identity binding. Upgrading to X.509 credentials would add stronger identity assurance.

#### No TLS

The server currently listens on plain HTTP. For production, it should either:

- Run behind a reverse proxy (Cloudflare, nginx) that terminates TLS.
- Use `axum-server` with `rustls` for native TLS termination.

#### REPL Limitations

The REPL does not currently listen for SSE events in the background. It only fetches messages on demand via `/messages`. A production REPL should spawn a background task that listens to the SSE stream and displays incoming messages in real time.

### Architecture Notes for Future Sessions

#### mls-rs Sync vs. Async

mls-rs defaults to **sync mode**. Async requires the `mls_build_async` cfg flag at compile time (set via `.cargo/config.toml` or `RUSTFLAGS`). We use sync mode and call MLS operations directly from async handlers. For CPU-heavy operations (large groups), consider wrapping in `tokio::task::spawn_blocking`.

#### mls-rs SQLite Version Alignment

`mls-rs-provider-sqlite` 0.21 depends on `rusqlite` 0.37. The server's direct `rusqlite` dependency must match (both use 0.37) to avoid `libsqlite3-sys` link conflicts in the workspace.

#### Group ID Mapping

MLS assigns its own internal group IDs (opaque bytes). The server assigns UUID strings as group IDs. The client maintains a `group_mapping.toml` file (per user, under `data_dir/users/<username>/`) that maps server UUIDs to MLS group IDs (hex-encoded). This mapping is essential — without it, the client cannot locate the correct MLS group state for encryption/decryption.

#### MLS State Rebuild on Each Operation

The current `MlsManager` rebuilds the `Client` object from persisted identity bytes on every operation. This works but is not efficient for high-throughput scenarios. A long-lived client (e.g., the REPL) could keep the `Client` in memory and only rebuild after restarts.

#### Protobuf over HTTP Pattern

Requests use `Content-Type: application/x-protobuf` with raw protobuf bytes in the body. The `proto_response` helper in `api.rs` encodes responses. The `decode_proto` helper decodes request bodies. This pattern is simple and avoids any JSON overhead.

### Dependencies to Watch

- `mls-rs` (0.53): Active development by AWS. API may change between minor versions. Pin carefully.
- `axum` (0.8): Stable. The `use<M>` precise capture syntax was needed for edition 2024 compatibility with the `impl IntoResponse` return type.
- `rusqlite` (0.37): Must stay aligned with `mls-rs-provider-sqlite`'s transitive dependency.
- `argon2` (0.5): Stable. Uses the `password-hash` crate ecosystem.

## 2026-02-15: Per-User MLS State Isolation

### Problem

When multiple users log in on the same client data directory (e.g., switching users in the REPL), MLS identity files (`mls_identity.bin`, `mls_signing_key.bin`) and the SQLite state database (`mls_state.db`) were shared. Logging in as user1 after user2 would load user2's MLS identity, causing "duplicate signature key, hpke key or identity found at index 0" errors when trying to add user2 to a group.

The `group_mapping.toml` file had the same issue — group mappings from one user would leak into another user's session.

### Fix

All per-user MLS state is now stored under `data_dir/users/<username>/`:

```
data_dir/
├── session.toml              # Current session (shared)
└── users/
    ├── alice/
    │   ├── mls_identity.bin
    │   ├── mls_signing_key.bin
    │   ├── mls_state.db
    │   └── group_mapping.toml
    └── bob/
        ├── mls_identity.bin
        ├── mls_signing_key.bin
        ├── mls_state.db
        └── group_mapping.toml
```

`MlsManager::new()` now creates and uses a per-user subdirectory. The REPL reloads the group mapping when a user logs in. `MlsManager::user_data_dir()` exposes the per-user path for callers that need it.

### What to Build Next

1. **Local sent message cache**: Store sent messages in plaintext locally so they appear in message history.
2. **Message cursor/watermark**: Track last-read sequence per group per user to avoid reprocessing.
3. **Multiple key packages**: Allow users to have N key packages so they can be added to multiple groups without re-uploading.
4. **Member removal**: Wire up `commit_builder().remove_member()` with a `DELETE /api/v1/groups/{id}/members/{uid}` endpoint.
5. **Background SSE in REPL**: Spawn a tokio task in the REPL that listens to SSE and displays incoming messages.
6. **TLS support**: Add rustls-based TLS termination or document reverse proxy setup.
7. **Message types in DB**: Add a `message_type` column (commit, application, proposal) to help clients filter.
8. **X.509 credentials**: Upgrade from BasicCredential for stronger identity assurance.
