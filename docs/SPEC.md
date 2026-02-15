# Conclave: Technical Specification

## 1. Overview

### 1.1 Purpose

Conclave is a minimalistic, self-hosted, end-to-end encrypted group messaging system. It uses the Messaging Layer Security (MLS) protocol (RFC 9420) to provide forward secrecy, post-compromise security, and efficient group key management. The server is a single binary with no external dependencies beyond a config file, designed to be trivially deployable on any infrastructure.

### 1.2 Design Principles

1. **Security**: MLS-based E2EE with no server-side access to plaintext. No compromises.
2. **Simplicity**: One code path for all messaging (2-person MLS groups for DMs, N-person MLS groups for rooms). Minimal feature surface. IRC-like operational simplicity.
3. **Efficiency**: Binary wire format (protobuf). Compact storage (SQLite). Small binary footprint.
4. **Deployability**: Single static binary. Single SQLite file. Single config file. No external services.

### 1.3 What Conclave Is

- A private, encrypted chat server and client suite
- A building block for third-party clients
- Designed for small-to-medium communities (think IRC server, not global platform)

### 1.4 What Conclave Is Not

- Not federated. Each server is an isolated community.
- Not a user discovery service. Users find each other out-of-band.

## 2. Architecture

Conclave uses a client-server architecture with MLS running on top of HTTP/2.

### 2.1 Workspace Layout

```
conclave/
├── Cargo.toml                          # Workspace root (resolver = "3", edition 2024)
├── proto/
│   └── conclave.proto                  # Protobuf wire format definitions
├── crates/
│   ├── conclave-proto/                 # Shared protobuf types (generated via prost)
│   │   ├── Cargo.toml
│   │   ├── build.rs                    # prost-build compilation
│   │   └── src/lib.rs
│   ├── conclave-server/                # Server binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs                 # Entry point, CLI, config loading, server startup
│   │       ├── config.rs               # ServerConfig (TOML deserialization)
│   │       ├── db.rs                   # SQLite database layer (all CRUD operations)
│   │       ├── auth.rs                 # Argon2id hashing, token generation, AuthUser extractor
│   │       ├── api.rs                  # axum router and all HTTP handlers
│   │       ├── state.rs                # AppState, SSE broadcast channel
│   │       └── error.rs                # Error enum with IntoResponse impl
│   └── conclave-client/                # Client binary
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                 # Entry point, clap subcommands, one-shot execution
│           ├── config.rs               # ClientConfig, SessionState persistence
│           ├── api.rs                  # ApiClient (reqwest HTTP wrapper)
│           ├── mls.rs                  # MlsManager (mls-rs operations with SQLite storage)
│           ├── repl.rs                 # Interactive REPL mode (rustyline)
│           └── error.rs                # Client error types
```

### 2.2 Technology Stack

| Component              | Choice                          | Crate(s)                                      |
|------------------------|---------------------------------|-----------------------------------------------|
| Language               | Rust (edition 2024)             |                                               |
| Server framework       | axum over HTTP/2                | `axum` (0.8)                                  |
| Client HTTP            | reqwest over HTTP/2             | `reqwest` (0.12)                              |
| Server-to-client push  | Server-Sent Events (SSE)        | `axum::response::sse`, `tokio-stream`         |
| Wire format            | Protocol Buffers                | `prost` (0.13), `prost-build` (0.13)          |
| MLS implementation     | mls-rs (sync mode)              | `mls-rs` (0.53), `mls-rs-crypto-openssl`      |
| MLS client storage     | SQLite (via mls-rs provider)    | `mls-rs-provider-sqlite` (0.21)               |
| Server database        | SQLite with WAL                 | `rusqlite` (0.37, bundled)                    |
| Password hashing       | Argon2id                        | `argon2` (0.5)                                |
| Authentication         | Opaque bearer tokens (256-bit)  | `rand`, `hex`                                 |
| Configuration          | TOML                            | `toml` (0.8), `serde`                         |
| Logging                | tracing                         | `tracing` (0.1), `tracing-subscriber` (0.3)   |
| CLI parsing            | clap (derive API)               | `clap` (4)                                    |
| Interactive REPL       | rustyline                       | `rustyline` (15)                              |
| Async runtime          | tokio                           | `tokio` (1, full features)                    |

### 2.3 Design Rationale

**Why axum + SSE instead of gRPC?** Cloudflare proxies gRPC by converting HTTP/2 to HTTP/1.1 gRPC-Web internally, which breaks bidirectional streaming and adds latency. SSE is a standard long-lived HTTP response that proxies through Cloudflare without issues.

**Why protobuf over HTTP instead of gRPC?** We use `prost` for message serialization without the gRPC transport layer. Requests are `POST` with `Content-Type: application/x-protobuf` bodies. This gives us schema-defined binary encoding with cross-language support while keeping the transport simple and proxy-friendly.

**Why sync mls-rs?** The mls-rs library defaults to synchronous mode. Async requires a `mls_build_async` cfg flag. Since MLS operations are CPU-bound cryptography, sync mode wrapped in tokio's blocking task pool is simpler and equally performant.

**Why opaque tokens instead of CWT/JWT?** For a single-server system with SQLite, a database lookup per request is negligible. Opaque tokens provide instant revocation without cryptographic complexity. CWT can be added later if needed.

## 3. Server

### 3.1 Configuration

Server configuration is loaded from a TOML file (default: `conclave-server.toml`). If the file does not exist, defaults are used.

```toml
# Address to bind the server to.
bind_address = "127.0.0.1:8443"

# Path to the SQLite database file.
database_path = "conclave.db"

# Session token lifetime in seconds (default: 7 days).
token_ttl_seconds = 604800
```

### 3.2 Database Schema

The server uses a single SQLite database with WAL journal mode and foreign keys enabled.

```sql
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE sessions (
    token TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    expires_at INTEGER NOT NULL
);

CREATE TABLE key_packages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    key_package_data BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    creator_id INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE group_members (
    group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    sender_id INTEGER NOT NULL REFERENCES users(id),
    mls_message BLOB NOT NULL,
    sequence_num INTEGER NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_messages_group_seq ON messages(group_id, sequence_num);

CREATE TABLE pending_welcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    group_name TEXT NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    welcome_data BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_pending_welcomes_user ON pending_welcomes(user_id);
```

### 3.3 Authentication

- **Registration**: Client sends username + password. Server hashes with Argon2id and stores.
- **Login**: Client sends username + password. Server verifies against stored hash, generates a 256-bit random opaque token (hex-encoded, 64 characters), stores it with an expiry, and returns it.
- **Authenticated requests**: Client sends `Authorization: Bearer <token>` header. Server validates against the `sessions` table, checking expiry. An `AuthUser` axum extractor handles this transparently for all protected endpoints.
- **Key packages**: One active key package per user. Uploading a new one replaces the old one. Key packages are consumed (deleted) when fetched by another user for group creation.

### 3.4 API Endpoints

All request/response bodies are protobuf-encoded (`Content-Type: application/x-protobuf`). Error responses use the `ErrorResponse` protobuf message.

#### Public Endpoints

| Method | Path                | Request Body      | Response Body      | Description                |
|--------|---------------------|-------------------|--------------------|----------------------------|
| POST   | `/api/v1/register`  | RegisterRequest   | RegisterResponse   | Register a new user        |
| POST   | `/api/v1/login`     | LoginRequest      | LoginResponse      | Login, receive auth token  |

#### Authenticated Endpoints

| Method | Path                              | Request Body          | Response Body               | Description                              |
|--------|-----------------------------------|-----------------------|-----------------------------|------------------------------------------|
| GET    | `/api/v1/me`                      | —                     | UserInfoResponse            | Get current user info                    |
| GET    | `/api/v1/users/{username}`        | —                     | UserInfoResponse            | Look up user by username                 |
| POST   | `/api/v1/key-packages`            | UploadKeyPackageReq   | UploadKeyPackageResp        | Upload MLS key package                   |
| GET    | `/api/v1/key-packages/{user_id}`  | —                     | GetKeyPackageResponse       | Fetch (consume) a user's key package     |
| POST   | `/api/v1/groups`                  | CreateGroupRequest    | CreateGroupResponse         | Create group, get member key packages    |
| GET    | `/api/v1/groups`                  | —                     | ListGroupsResponse          | List user's groups                       |
| POST   | `/api/v1/groups/{id}/commit`      | UploadCommitRequest   | UploadCommitResponse        | Upload MLS commit + welcome messages     |
| POST   | `/api/v1/groups/{id}/messages`    | SendMessageRequest    | SendMessageResponse         | Send encrypted message                   |
| GET    | `/api/v1/groups/{id}/messages`    | —  (`?after=&limit=`) | GetMessagesResponse         | Fetch messages (paginated by seq num)    |
| GET    | `/api/v1/welcomes`                | —                     | ListPendingWelcomesResponse | List pending group invitations           |
| POST   | `/api/v1/welcomes/{id}/accept`    | —                     | 204 No Content              | Accept and delete a pending welcome      |
| GET    | `/api/v1/events`                  | —                     | SSE stream                  | Real-time event notifications            |

### 3.5 SSE Events

The `/api/v1/events` endpoint provides a Server-Sent Events stream. Events are hex-encoded protobuf `ServerEvent` messages. Each event is targeted at specific user IDs; the server filters so clients only receive their own events.

Event types:
- **NewMessageEvent**: New message in a group (group_id, sequence_num, sender_id).
- **GroupUpdateEvent**: Group state changed, e.g., a commit was uploaded (group_id, update_type).
- **WelcomeEvent**: User was invited to a group (group_id, group_name).

The server uses a `tokio::sync::broadcast` channel internally to fan out events.

## 4. Client

### 4.1 Configuration

Client configuration is loaded from a TOML file (default: `conclave-client.toml`).

```toml
# Server base URL.
server_url = "http://127.0.0.1:8443"

# Local data directory for MLS state, session, and group mappings.
data_dir = "/home/user/.local/share/conclave"
```

If `data_dir` is not specified, it defaults to the platform-appropriate application data directory (via the `directories` crate).

### 4.2 Local Storage

The client persists the following files in `data_dir`:

| File                  | Format        | Contents                                                    |
|-----------------------|---------------|-------------------------------------------------------------|
| `session.toml`        | TOML          | Auth token, user ID, username                               |
| `mls_identity.bin`    | MLS codec     | Serialized `SigningIdentity` (public key + credential)      |
| `mls_signing_key.bin` | Raw bytes     | `SignatureSecretKey` (private key material)                  |
| `mls_state.db`        | SQLite        | mls-rs group state, key packages, PSKs (via mls-rs-provider-sqlite) |
| `group_mapping.toml`  | TOML          | Map of server group UUID to MLS group ID (hex)              |

### 4.3 MLS Integration

- **Cipher suite**: `CURVE25519_AES128` (MLS cipher suite 1)
- **Crypto backend**: OpenSSL via `mls-rs-crypto-openssl`
- **Identity**: `BasicCredential` with username bytes (suitable for the current trust model; X.509 can be added later)
- **Storage**: SQLite-backed via `mls-rs-provider-sqlite` with `FileConnectionStrategy`

#### MLS Operations

| Operation           | mls-rs API                                                        |
|---------------------|-------------------------------------------------------------------|
| Generate identity   | `cipher_suite.signature_key_generate()` → `SigningIdentity`       |
| Generate key pkg    | `client.generate_key_package_message()`                           |
| Create group        | `client.create_group()` → `group.commit_builder().add_member()`   |
| Join group          | `client.join_group(None, &welcome_msg, None)`                     |
| Encrypt message     | `group.encrypt_application_message(plaintext, auth_data)`         |
| Decrypt message     | `group.process_incoming_message(msg)` → `ReceivedMessage`         |
| Persist state       | `group.write_to_storage()`                                        |
| Load group          | `client.load_group(&group_id_bytes)`                              |

### 4.4 CLI Modes

#### One-Shot Commands

```
conclave-client [-c config.toml] <command>

Commands:
  register      -u <username> -p <password>       Register a new account
  login         -u <username> -p <password>       Login and store session
  keygen                                          Generate and upload MLS key package
  create-group  -n <name> -m <user1,user2,...>    Create encrypted group
  groups                                          List joined groups
  join                                            Accept pending group invitations
  send          -g <group_id> -m <message>        Send encrypted message
  messages      -g <group_id>                     Fetch and decrypt messages
```

#### Interactive REPL

Running `conclave-client` with no subcommand enters interactive mode. Commands are prefixed with `/`:

```
/register <username> <password>
/login <username> <password>
/me
/keygen
/create <group_name> <member1,member2,...>
/groups
/join
/send <group_id> <message>
/messages <group_id>
/quit
```

## 5. Protocol Flows

### 5.1 Registration and Setup

```
Client                              Server
  |                                   |
  |--- POST /register (user, pass) -->|  Store user with Argon2id hash
  |<-- RegisterResponse (user_id) ----|
  |                                   |
  |--- POST /login (user, pass) ----->|  Verify hash, generate token
  |<-- LoginResponse (token, uid) ----|
  |                                   |
  |  [Generate MLS SigningIdentity]   |
  |  [Store keys locally in SQLite]   |
  |                                   |
  |--- POST /key-packages (kp) ------>|  Store key package blob
  |<-- OK ----------------------------|
```

### 5.2 Group Creation

```
Alice                               Server                              Bob
  |                                   |                                   |
  |--- POST /groups (name, [bob]) --->|  Consume Bob's key package        |
  |<-- CreateGroupResponse            |                                   |
  |    (group_id, {bob: kp_bytes})    |                                   |
  |                                   |                                   |
  |  [MLS: create_group()]            |                                   |
  |  [MLS: commit_builder()           |                                   |
  |        .add_member(bob_kp)        |                                   |
  |        .build()]                  |                                   |
  |  [MLS: apply_pending_commit()]    |                                   |
  |                                   |                                   |
  |--- POST /groups/{id}/commit ----->|  Store commit as message seq 1    |
  |    (commit, welcomes, group_info) |  Store welcome for Bob            |
  |<-- OK                             |  Add Bob to group_members         |
  |                                   |  Notify Bob via SSE               |
  |                                   |                                   |
  |                                   |<--- GET /welcomes --------------- |
  |                                   |---- PendingWelcome (welcome) ---->|
  |                                   |                                   |
  |                                   |                [MLS: join_group() |
  |                                   |                 via welcome_msg]  |
```

### 5.3 Messaging

```
Alice                               Server                              Bob
  |                                   |                                   |
  |  [MLS: encrypt_application_       |                                   |
  |        message(plaintext)]        |                                   |
  |                                   |                                   |
  |--- POST /groups/{id}/messages --->|  Store encrypted blob             |
  |    (mls_message bytes)            |  Assign sequence number           |
  |<-- SendMessageResponse (seq) -----|  SSE: NewMessageEvent to Bob      |
  |                                   |                                   |
  |                                   |<--- GET /groups/{id}/messages --- |
  |                                   |---- GetMessagesResponse --------->|
  |                                   |                                   |
  |                                   |                [MLS: process_     |
  |                                   |                 incoming_message] |
  |                                   |                → plaintext        |
```

## 6. Security Considerations

- **E2EE**: The server never sees plaintext. All application messages are MLS `PrivateMessage` ciphertexts. The server stores and forwards opaque blobs.
- **Forward secrecy**: Provided by MLS key ratcheting. Compromising current keys does not reveal past messages.
- **Post-compromise security**: MLS commit operations rotate key material, recovering security after a compromise.
- **Password storage**: Argon2id with random salts. No plaintext passwords stored.
- **Token security**: 256-bit cryptographically random tokens from `OsRng`. Tokens have configurable expiry.
- **MLS identity keys**: Stored locally on the client filesystem. The `mls_signing_key.bin` file contains the private key and must be protected by filesystem permissions.
- **Trust model**: Currently uses `BasicCredential` with `BasicIdentityProvider`, which does not validate identities. This is suitable for closed communities. X.509 credentials can be added for stronger identity assurance.

## 7. Protobuf Schema

The wire format is defined in `proto/conclave.proto`. All messages are in the `conclave` package. MLS messages are carried as opaque `bytes` fields — the server does not interpret their contents.

Key message types: `RegisterRequest/Response`, `LoginRequest/Response`, `UploadKeyPackageRequest/Response`, `GetKeyPackageResponse`, `CreateGroupRequest/Response`, `GroupInfo`, `ListGroupsResponse`, `UploadCommitRequest/Response`, `SendMessageRequest/Response`, `StoredMessage`, `GetMessagesResponse`, `PendingWelcome`, `ListPendingWelcomesResponse`, `ServerEvent` (oneof: `NewMessageEvent`, `GroupUpdateEvent`, `WelcomeEvent`), `ErrorResponse`, `UserInfoResponse`.
