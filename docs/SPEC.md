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

Conclave uses a client-server architecture with MLS running on top of HTTP/2. Shared client logic (API client, MLS manager, config, message store, command parsing) lives in the `conclave-lib` library crate, consumed by both the CLI/TUI (`conclave-cli`) and GUI (`conclave-gui`) client binaries.

### 2.1 Workspace Layout

```
conclave/
├── Cargo.toml                          # Workspace root (resolver = "3", edition 2024)
├── AGENT.md                            # Agent directives for AI assistants
├── proto/
│   └── conclave.proto                  # Protobuf wire format definitions
├── crates/
│   ├── conclave-proto/                 # Shared protobuf types (generated via prost)
│   │   ├── Cargo.toml
│   │   ├── build.rs                    # prost-build compilation
│   │   └── src/lib.rs
│   ├── conclave-server/                # Server binary + library
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── lib.rs                  # Re-exports all modules for integration testing
│   │   │   ├── main.rs                 # Entry point, CLI, config loading, TLS/plaintext server startup
│   │   │   ├── config.rs              # ServerConfig (TOML deserialization, TLS options)
│   │   │   ├── db.rs                  # SQLite database layer (all CRUD operations)
│   │   │   ├── auth.rs                # Argon2id hashing, token generation, AuthUser extractor
│   │   │   ├── api.rs                 # axum router and all HTTP handlers
│   │   │   ├── state.rs               # AppState, SSE broadcast channel
│   │   │   └── error.rs               # Error enum with IntoResponse impl
│   │   └── tests/
│   │       └── api_tests.rs           # Integration tests (tower::oneshot)
│   ├── conclave-lib/                   # Shared library (used by CLI and GUI)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs                  # Re-exports all modules
│   │       ├── api.rs                  # ApiClient (reqwest HTTP wrapper, TLS config)
│   │       ├── mls.rs                  # MlsManager (mls-rs operations with SQLite storage)
│   │       ├── config.rs              # ClientConfig, SessionState, group mapping I/O, key package generation
│   │       ├── error.rs                # Client error types
│   │       ├── state.rs               # Room, RoomMember, DisplayMessage, ConnectionStatus
│   │       ├── store.rs               # SQLite-backed message history persistence
│   │       └── command.rs             # Command enum and parser
│   ├── conclave-cli/                   # CLI/TUI client binary
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs                 # Entry point, clap subcommands, one-shot + TUI
│   │       ├── error.rs                # TUI-specific error types
│   │       └── tui/
│   │           ├── mod.rs             # Main event loop (crossterm + SSE + reconnection)
│   │           ├── input.rs           # InputLine editor with cursor movement and history
│   │           ├── render.rs          # Terminal drawing (messages, status line, input)
│   │           ├── commands.rs        # IRC-style command execution
│   │           └── events.rs          # SSE event decoding and handling
│   └── conclave-gui/                   # GUI client binary (iced)
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs                 # Entry point, iced application launch
│           ├── app.rs                  # Conclave application struct, Message enum, update/view
│           ├── subscription.rs        # SSE subscription, tick timer
│           ├── screen/
│           │   ├── mod.rs             # Screen enum (Login, Dashboard)
│           │   ├── login.rs           # Login/register screen
│           │   └── dashboard.rs       # Main chat interface
│           ├── widget/
│           │   ├── mod.rs             # Element type alias, re-exports
│           │   └── message_view.rs    # Chat message rendering widget
│           └── theme/
│               ├── mod.rs             # Theme struct, Base impl, colors
│               ├── button.rs          # Button styles
│               ├── container.rs       # Container styles
│               ├── text.rs            # Text styles
│               ├── text_input.rs      # Input field styles
│               └── scrollable.rs      # Scrollbar styles
```

### 2.2 Technology Stack

| Component              | Choice                          | Crate(s)                                      |
|------------------------|---------------------------------|-----------------------------------------------|
| Language               | Rust (edition 2024)             |                                               |
| Server framework       | axum over HTTP/2                | `axum` (0.8)                                  |
| Server TLS             | rustls (optional)               | `axum-server` (0.8), `rustls-pemfile`         |
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
| Interactive TUI        | crossterm + SSE                 | `crossterm` (0.28), `reqwest-eventsource`     |
| GUI framework          | iced                            | `iced` (0.14)                                 |
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

# TLS certificate and key paths (PEM format).
# If both are set, the server listens with TLS (HTTPS).
# If omitted, the server listens on plain HTTP.
# tls_cert_path = "/path/to/cert.pem"
# tls_key_path = "/path/to/key.pem"
```

### 3.1.1 Transport Security

The server supports two transport modes:

1. **Plain HTTP** (default): When `tls_cert_path` and `tls_key_path` are not set, the server listens on plain HTTP. This mode is suitable when running behind a TLS-terminating reverse proxy (e.g., nginx, Cloudflare).

2. **Native TLS**: When both `tls_cert_path` and `tls_key_path` are set, the server uses `axum-server` with `rustls` to serve HTTPS directly. The certificate and key must be in PEM format. This mode is suitable for direct exposure without a reverse proxy.

Clients validate the server's TLS certificate by default. For development with self-signed certificates, clients can set `accept_invalid_certs = true` in their configuration.

### 3.2 Database Schema

The server uses a single SQLite database with WAL journal mode and foreign keys enabled.

```sql
CREATE TABLE users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT UNIQUE NOT NULL,
    alias TEXT,
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
    is_last_resort INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE groups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_name TEXT UNIQUE,
    alias TEXT,
    mls_group_id TEXT,
    creator_id INTEGER NOT NULL REFERENCES users(id),
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE group_members (
    group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (group_id, user_id)
);

CREATE TABLE messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    sender_id INTEGER NOT NULL REFERENCES users(id),
    mls_message BLOB NOT NULL,
    sequence_num INTEGER NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    UNIQUE(group_id, sequence_num)
);

CREATE TABLE pending_welcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    group_alias TEXT,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    welcome_data BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE INDEX idx_pending_welcomes_user ON pending_welcomes(user_id);

CREATE TABLE group_infos (
    group_id INTEGER PRIMARY KEY REFERENCES groups(id) ON DELETE CASCADE,
    group_info_data BLOB NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
```

### 3.3 Authentication

- **Registration**: Client sends username + password. Server validates the username against `^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$` (ASCII alphanumeric start, max 64 chars, no control characters or Unicode homoglyphs) and requires a minimum password length of 8 characters. An optional alias (display name) can be provided, subject to validation: max 64 characters, no ASCII control characters (0x00-0x1F, 0x7F). Password is hashed with Argon2id and stored.
- **Login**: Client sends username + password. Server verifies against stored hash, generates a 256-bit random opaque token (hex-encoded, 64 characters), stores it with an expiry, and returns it. To prevent username enumeration via timing analysis, the server runs `verify_password()` against a precomputed dummy Argon2id hash when the requested username does not exist, ensuring both code paths have equivalent computational profiles.
- **Authenticated requests**: Client sends `Authorization: Bearer <token>` header. Server validates against the `sessions` table, checking expiry. An `AuthUser` axum extractor handles this transparently for all protected endpoints.
- **Key packages**: Each user maintains up to 10 regular key packages plus 1 last-resort package. Regular packages are consumed FIFO (oldest first) and deleted on consumption. The last-resort package is returned but never deleted when all regular packages are exhausted, ensuring the user is always reachable (RFC 9420 Section 16.6). Uploading a new last-resort package replaces the previous one. Clients pre-publish 5 regular + 1 last-resort on registration/login/reset and replenish after each consumption. The server validates key package uploads by checking the MLS wire format header (version must be MLS 1.0, wire format must be `mls_key_package`) per RFC 9420 Section 6.
- **Rate limiting**: The `GET /api/v1/key-packages/{user_id}` endpoint is rate-limited to 10 requests per minute per target user to prevent key package exhaustion attacks.

### 3.4 API Endpoints

All request/response bodies are protobuf-encoded (`Content-Type: application/x-protobuf`). Error responses use the `ErrorResponse` protobuf message.

#### Public Endpoints

| Method | Path                | Request Body      | Response Body      | Description                |
|--------|---------------------|-------------------|--------------------|----------------------------|
| POST   | `/api/v1/register`  | RegisterRequest   | RegisterResponse   | Register a new user        |
| POST   | `/api/v1/login`     | LoginRequest      | LoginResponse      | Login, receive auth token  |

#### Authenticated Endpoints

| Method | Path                                | Request Body          | Response Body               | Description                              |
|--------|-------------------------------------|-----------------------|-----------------------------|------------------------------------------|
| POST   | `/api/v1/logout`                    | —                     | —                           | Revoke session token                     |
| GET    | `/api/v1/me`                        | —                     | UserInfoResponse            | Get current user info                    |
| PATCH  | `/api/v1/me`                        | UpdateProfileRequest  | UpdateProfileResponse       | Update user alias                        |
| GET    | `/api/v1/users/{username}`          | —                     | UserInfoResponse            | Look up user by username                 |
| POST   | `/api/v1/key-packages`              | UploadKeyPackageReq   | UploadKeyPackageResp        | Upload MLS key package(s)                |
| GET    | `/api/v1/key-packages/{user_id}`    | —                     | GetKeyPackageResponse       | Fetch (consume) a user's key package     |
| POST   | `/api/v1/groups`                    | CreateGroupRequest    | CreateGroupResponse         | Create group, get member key packages    |
| PATCH  | `/api/v1/groups/{id}`               | UpdateGroupRequest    | UpdateGroupResponse         | Update group alias/name (creator only)   |
| GET    | `/api/v1/groups`                    | —                     | ListGroupsResponse          | List user's groups                       |
| POST   | `/api/v1/groups/{id}/invite`        | InviteToGroupRequest  | InviteToGroupResponse       | Invite members, get their key packages   |
| POST   | `/api/v1/groups/{id}/commit`        | UploadCommitRequest   | UploadCommitResponse        | Upload MLS commit + welcome messages     |
| POST   | `/api/v1/groups/{id}/messages`      | SendMessageRequest    | SendMessageResponse         | Send encrypted message                   |
| GET    | `/api/v1/groups/{id}/messages`      | —  (`?after=&limit=`) | GetMessagesResponse         | Fetch messages (paginated by seq num)    |
| POST   | `/api/v1/groups/{id}/remove`        | RemoveMemberRequest   | RemoveMemberResponse        | Remove a member from the group           |
| POST   | `/api/v1/groups/{id}/leave`         | LeaveGroupRequest     | LeaveGroupResponse          | Leave a group                            |
| GET    | `/api/v1/groups/{id}/group-info`    | —                     | GetGroupInfoResponse        | Get stored GroupInfo for external commit  |
| POST   | `/api/v1/groups/{id}/external-join` | ExternalJoinRequest   | ExternalJoinResponse        | Rejoin group via external commit         |
| POST   | `/api/v1/reset-account`             | —                     | ResetAccountResponse        | Clear key packages for account reset     |
| GET    | `/api/v1/welcomes`                  | —                     | ListPendingWelcomesResponse | List pending group invitations           |
| POST   | `/api/v1/welcomes/{id}/accept`      | —                     | 204 No Content              | Accept and delete a pending welcome      |
| GET    | `/api/v1/events`                    | —                     | SSE stream                  | Real-time event notifications            |

#### Endpoint Behavior Notes

- **`POST /api/v1/key-packages`**: Supports both single-upload (via `key_package_data` field) and batch-upload (via `entries` repeated field with `KeyPackageEntry` messages containing `data` and `is_last_resort` flag). All uploads are validated for MLS wire format correctness (version 0x0001, wire format 0x0005 for `mls_key_package` per RFC 9420 Section 6). Uploading a last-resort package replaces any existing last-resort package for that user.
- **`GET /api/v1/key-packages/{user_id}`**: Rate-limited to 10 requests per minute per target user. Consumes the oldest regular key package (FIFO). Falls back to the last-resort package (without deleting it) when no regular packages remain.
- **`POST /api/v1/groups/{id}/commit`**: All database operations (member additions, welcome storage, group info update, commit message storage) are performed atomically within a single SQLite savepoint transaction. SSE notifications are sent only after the transaction commits. Newly added members receive `WelcomeEvent`; existing members (excluding the sender and newly added members) receive `GroupUpdateEvent`. If the request includes a non-empty `mls_group_id`, it is stored in the `groups` table (only set once, on group creation).
- **`POST /api/v1/groups/{id}/leave`**: Accepts an optional `commit_message` and `group_info` in the request body. If a commit message is provided, it is stored as a group message so remaining members can process the MLS removal and advance their epoch. If group info is provided, it is stored for potential external rejoin. The user is then removed from the server's group membership, and remaining members are notified via `MemberRemovedEvent`.
- **`POST /api/v1/groups/{id}/external-join`**: Requires the group to exist and have a stored `GroupInfo` (set by prior `upload_commit` or `remove` operations). This prevents arbitrary users from joining groups they were never associated with — only groups whose authorized members have published a GroupInfo can be externally joined. The external commit is stored as a group message and existing members receive an `IdentityResetEvent`. If the request includes a non-empty `mls_group_id`, it is stored in the `groups` table (only set once, preserving the original).

### 3.5 SSE Events

The `/api/v1/events` endpoint provides a Server-Sent Events stream. Events are hex-encoded protobuf `ServerEvent` messages. Each event is targeted at specific user IDs; the server filters so clients only receive their own events.

Event types:
- **NewMessageEvent**: New message in a group (group_id, sequence_num, sender_id).
- **GroupUpdateEvent**: Group state changed (group_id, update_type). Emitted for MLS commits (`update_type: "commit"`) and member profile changes (`update_type: "member_profile"`). Profile updates are broadcast to all co-members when a user changes their alias via `PATCH /api/v1/me`.
- **WelcomeEvent**: User was invited to a group (group_id, group_alias).
- **MemberRemovedEvent**: A member was removed or left a group (group_id, removed_user_id, removed_username). Sent to both remaining members and the removed user.
- **IdentityResetEvent**: A member reset their encryption identity via external rejoin (group_id, username). Sent to other group members when a user performs an account reset. Clients display a warning that the user's encryption keys have changed.
- **lagged** (transport-level): Sent when the client's SSE stream falls behind the broadcast channel buffer. The `event` field is `"lagged"` and the `data` field contains the number of dropped events as a string. Clients should treat this as a signal to re-fetch group state. This is not a protobuf `ServerEvent` — it is a raw SSE event emitted by the transport layer.

The server uses a `tokio::sync::broadcast` channel internally to fan out events.

## 4. Client

### 4.1 Configuration

Client configuration is loaded from a TOML file (default: `conclave-cli.toml`).

```toml
# Local data directory for MLS state, session, and group mappings.
data_dir = "/home/user/.local/share/conclave"

# Accept invalid TLS certificates (e.g., self-signed). Default: false.
# accept_invalid_certs = false
```

If `data_dir` is not specified, it defaults to the platform-appropriate application data directory (via the `directories` crate).

The server URL is not part of the configuration file. Instead, it is specified during login/registration and persisted in the session state (`session.toml`). This allows users to connect to different servers without modifying configuration files. If no URL scheme is provided (e.g., `example.com:8443`), the client automatically prepends `https://`.

The client validates the server's TLS certificate by default when connecting over HTTPS. For development or testing with self-signed certificates, set `accept_invalid_certs = true`.

HTTP error messages include the full cause chain for easier debugging (e.g., "HTTP error: error sending request: ... connection refused" instead of just "HTTP error: builder error").

### 4.2 Local Storage

The client persists the following files in `data_dir`:

| File                  | Format        | Contents                                                    |
|-----------------------|---------------|-------------------------------------------------------------|
| `session.toml`        | TOML          | Server URL, auth token, user ID, username                   |
| `mls_identity.bin`    | MLS codec     | Serialized `SigningIdentity` (public key + credential)      |
| `mls_signing_key.bin` | Raw bytes     | `SignatureSecretKey` (private key material)                  |
| `mls_state.db`        | SQLite        | mls-rs group state, key packages, PSKs (via mls-rs-provider-sqlite) |
| `group_mapping.toml`  | TOML          | Local cache of server group ID (i64) to MLS group ID (hex string). Used as fallback by one-shot CLI commands; TUI/GUI build mapping from server on login. |
| `message_history.db`  | SQLite        | Decrypted message history and per-room sequence tracking. Messages store `sender_id` to enable render-time display name resolution from room member lists. |

### 4.3 MLS Integration

- **Cipher suite**: `CURVE448_CHACHA` (MLS cipher suite 6 — X448, ChaCha20-Poly1305, SHA-512, Ed448, 256-bit security)
- **Crypto backend**: OpenSSL via `mls-rs-crypto-openssl`
- **Identity**: `BasicCredential` with user ID bytes (i64, big-endian, 8 bytes). Using integer IDs instead of usernames ensures credential stability when display names change.
- **Storage**: SQLite-backed via `mls-rs-provider-sqlite` with `FileConnectionStrategy`

#### Epoch Retention

MLS groups advance through epochs on each commit (member add/remove, key rotation, external rejoin). The mls-rs default epoch retention is 3, which is too tight for real-world offline periods. Conclave configures `with_max_epoch_retention(16)` on the group state storage, allowing clients to decrypt messages from up to 16 prior epochs. This means a client can be offline through 16 group state transitions (commits) and still catch up on missed messages. RFC 9420 does not specify a recommended epoch retention value — this is left to implementations.

Regular application messages (chat) do not advance the epoch. A client can be offline through an unlimited number of chat messages within the same epoch.

#### Decryption Error Handling

`decrypt_message()` returns a `DecryptedMessage` enum with four variants:
- `Application(Vec<u8>)`: Successfully decrypted application message.
- `Commit(CommitInfo)`: Successfully processed commit with roster change details.
- `None`: Expected non-error condition (e.g., processing own commit after welcome).
- `Failed(String)`: Decryption failure with reason (epoch evicted, key missing, invalid signature, etc.).

When `Failed` is returned, both CLI and GUI clients display a system message notifying the user of the undecryptable message (with sequence number and reason) and continue processing subsequent messages. The `last_seen_seq` is still advanced — permanently undecryptable messages cannot be retried, so blocking on them would cause infinite retry loops. Users can `/reset` to rejoin the group with fresh state if needed.

#### MLS Operations

| Operation           | mls-rs API                                                        |
|---------------------|-------------------------------------------------------------------|
| Generate identity   | `cipher_suite.signature_key_generate()` → `SigningIdentity`       |
| Generate key pkg    | `client.generate_key_package_message()`                           |
| Create group        | `client.create_group()` → `group.commit_builder().add_member()`   |
| Join group          | `client.join_group(None, &welcome_msg, None)`                     |
| Encrypt message     | `group.encrypt_application_message(plaintext, auth_data)`         |
| Decrypt message     | `group.process_incoming_message(msg)` → `ReceivedMessage`         |
| Remove member       | `group.commit_builder().remove_member(index).build()`             |
| Rotate keys         | `group.commit_builder().build()` (empty commit advances epoch)    |
| External rejoin     | `client.external_commit_builder().with_removal(index).build(gi)`  |
| Get group info      | `group.group_info_message_allowing_ext_commit(true)`              |
| Persist state       | `group.write_to_storage()`                                        |
| Load group          | `client.load_group(&group_id_bytes)`                              |

### 4.4 CLI Modes

#### One-Shot Commands

```
conclave-cli [-c config.toml] <command>

Commands:
  register      -s <server> -u <username> -p <password>  Register a new account
  login         -s <server> -u <username> -p <password>  Login and store session
  keygen                                          Generate and upload MLS key package
  create-group  -n <name> -m <user1,user2,...>    Create encrypted group
  groups                                          List joined groups
  join                                            Accept pending group invitations
  send          -g <group_id> -m <message>        Send encrypted message
  messages      -g <group_id>                     Fetch and decrypt messages
```

#### Interactive TUI

Running `conclave-cli` with no subcommand enters the IRC-style interactive TUI. Commands are prefixed with `/`; plain text sends to the active room.

```
/register <server> <user> <pass>  Register a new account
/login <server> <user> <pass>    Login to the server
/keygen                       Generate and upload a key package
/create <name> <user1,user2>  Create a room with members
/join                         Accept pending invitations
/join <room>                  Switch to a room
/invite <user1,user2>         Invite to the active room
/kick <username>              Remove a member from the room
/leave                        Leave the room (MLS removal)
/part                         Switch away without leaving
/rotate                       Rotate keys (forward secrecy)
/reset                        Reset account and rejoin groups
/info                         Show MLS group details
/rooms                        List your rooms
/unread                       Check rooms for new messages
/logout                       Logout and revoke session
/who                          List members of active room
/msg <room> <text>            Send to a room without switching
/me                           Show current user info
/help                         Show help
/quit                         Exit
```

### 4.5 GUI Client

The GUI client (`conclave-gui`) is built with [iced](https://iced.rs/) 0.14 and provides a graphical alternative to the TUI. It shares all core logic (API client, MLS manager, config, message store, command parsing) via `conclave-lib`.

**Architecture**: Elm-style (model → update → view) with iced's `application()` entry point, custom `Theme` implementing `iced::theme::Base`, and SSE subscriptions via `iced::Subscription::run_with()`.

**Screens**:
- **Login**: Centered card with server URL, username, password fields, and login/register toggle.
- **Dashboard**: Three-panel layout — sidebar (room list with unread counts, connection status, logout), title bar (room name, member count), scrollable message area, and chat input. Supports all `/` commands from the TUI.

**Async integration**: All API calls use `Task::perform()`. MLS crypto operations (sync) are wrapped in `tokio::task::spawn_blocking` inside `Task::perform()`.

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
- **Transport security**: The server supports native TLS via rustls. Clients validate server certificates by default. For deployments behind a TLS-terminating reverse proxy, the server can run in plain HTTP mode.
- **Password storage**: Argon2id with random salts. No plaintext passwords stored.
- **Token security**: 256-bit cryptographically random tokens from `OsRng`. Tokens have configurable expiry.
- **MLS identity keys**: Stored locally on the client filesystem. The `mls_signing_key.bin` file contains the private key and must be protected by filesystem permissions.
- **Username validation**: Usernames are restricted to ASCII alphanumeric characters, underscores, hyphens, and periods (`^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$`). This prevents control characters, Unicode homoglyphs, and whitespace-only usernames that could be used for impersonation or display attacks.
- **Timing attack mitigation**: The login endpoint runs `verify_password()` against a precomputed dummy Argon2id hash when the requested username does not exist. This ensures both the valid-user and invalid-user code paths have equivalent computational profiles, preventing username enumeration via timing analysis.
- **Key package validation**: The server validates all uploaded key packages for MLS wire format correctness (MLS version 1.0 header, `mls_key_package` wire format type) per RFC 9420 Section 6. This prevents malformed or non-MLS data from being stored and later causing failures during group creation or invitation on the inviter's client.
- **Key package exhaustion protection**: The `GET /api/v1/key-packages/{user_id}` endpoint is rate-limited to prevent an attacker from draining a user's regular key packages, which would force fallback to the reusable last-resort package (with associated reuse risks per RFC 9420 Section 16.8).
- **External join authorization**: The external join endpoint requires a stored `GroupInfo` to exist for the target group. Since `GroupInfo` is only stored by authorized members via `upload_commit` or `remove` operations, this prevents arbitrary users from joining groups they have no association with.
- **Transactional integrity**: The `upload_commit` endpoint performs all database mutations (member additions, welcome storage, group info updates, message storage) within a single SQLite savepoint transaction. This ensures atomicity — a crash mid-operation cannot leave the database in an inconsistent state.
- **Trust model**: Currently uses `BasicCredential` with `BasicIdentityProvider`, which does not validate identities. This is suitable for closed communities. X.509 credentials can be added for stronger identity assurance.

## 7. Protobuf Schema

The wire format is defined in `proto/conclave.proto`. All messages are in the `conclave` package. MLS messages are carried as opaque `bytes` fields — the server does not interpret their contents.

Key message types: `RegisterRequest/Response`, `LoginRequest/Response`, `UploadKeyPackageRequest/Response` (with `KeyPackageEntry` for batch uploads containing `data` and `is_last_resort` fields), `GetKeyPackageResponse`, `CreateGroupRequest/Response`, `GroupInfo` (with `mls_group_id` for server-side group mapping), `GroupMember` (with `user_id`, `username`, `alias`), `ListGroupsResponse`, `InviteToGroupRequest/Response`, `UploadCommitRequest/Response` (with `mls_group_id` set on group creation), `SendMessageRequest/Response`, `StoredMessage`, `GetMessagesResponse`, `PendingWelcome` (with `group_alias`), `ListPendingWelcomesResponse`, `RemoveMemberRequest/Response` (with `commit_message` and `group_info`), `LeaveGroupRequest/Response` (with `commit_message` and `group_info` for MLS leave commits and external rejoin support), `GetGroupInfoResponse`, `ExternalJoinRequest/Response` (with `mls_group_id` set on rejoin), `ResetAccountResponse`, `UpdateProfileRequest/Response`, `UpdateGroupRequest/Response`, `ServerEvent` (oneof: `NewMessageEvent`, `GroupUpdateEvent`, `WelcomeEvent` (with `group_alias`), `MemberRemovedEvent`, `IdentityResetEvent`), `ErrorResponse`, `UserInfoResponse`.
