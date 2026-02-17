# Conclave Work Log

## 2026-02-17: RFC 9420-Compliant Key Package Lifecycle

### What Changed

Full RFC 9420 key package lifecycle overhaul: multiple key packages per user, last-resort key packages, explicit lifetime, and batch upload. This resolves the "Single Key Package" limitation from the initial implementation.

#### Problem

The original implementation stored at most 1 key package per user and deleted it on consumption. This caused three issues:
1. **Concurrent group invitations fail**: If two users try to invite a third simultaneously, only one gets the key package.
2. **Lost key package on failure**: If a group creation fails mid-flight after consuming the key package, it's gone.
3. **No fallback**: When all key packages are consumed, the user is unreachable until they manually run `/keygen`.

RFC 9420 §16.8 requires key packages to never be reused and recommends pre-publishing multiple packages. §7.2/§7.3 require explicit `not_before`/`not_after` lifetime fields.

#### Design

1. **Multiple key packages per user**: Pre-publish 5 regular packages + 1 last-resort. Regular packages are consumed FIFO (oldest first). Server enforces a cap of 10 regular packages per user.
2. **Last-resort key package** (RFC 9420 §16.6): A permanently-stored fallback that is never deleted on consumption. When all regular packages are exhausted, the server returns the last-resort package without deleting it, so the user is always reachable. Uploading a new last-resort replaces the previous one (max 1).
3. **Explicit key package lifetime**: 90-day `not_before`/`not_after` via `.key_package_lifetime(Duration::from_secs(90 * 24 * 3600))` on the mls-rs client builder.
4. **Batch upload**: New `UploadKeyPackageRequest.entries` field carries multiple `KeyPackageEntry { data, is_last_resort }` in a single request. Legacy single-upload path preserved for backward compatibility.
5. **Auto-replenishment**: After consuming a key package (welcome processing), clients upload 1 regular replacement. On session restore, login, `/keygen`, and `/reset`, clients upload the full batch (1 last-resort + 5 regular).

#### Changes

**Proto** (`proto/conclave/v1/conclave.proto`):
- Added `KeyPackageEntry` message with `data` and `is_last_resort` fields.
- Added `repeated KeyPackageEntry entries` field to `UploadKeyPackageRequest`.

**Server DB** (`conclave-server/src/db.rs`):
- Added `is_last_resort INTEGER NOT NULL DEFAULT 0` column to `key_packages` table with idempotent `ALTER TABLE` migration.
- `store_key_package(user_id, data, is_last_resort)`: Accumulates regular packages up to cap (10), replaces last-resort (max 1).
- `consume_key_package(user_id)`: Prefers regular packages (FIFO), falls back to last-resort WITHOUT deleting.
- Added `count_key_packages(user_id) -> (regular, last_resort)` helper.
- New tests: `test_key_package_accumulate`, `test_last_resort_key_package`, `test_key_package_cap`, `test_last_resort_replacement`.

**Server API** (`conclave-server/src/api.rs`):
- `upload_key_package` handler supports batch path (`req.entries` non-empty) and legacy single-upload path.
- New integration tests: `test_batch_upload_and_ordered_consumption`, `test_last_resort_not_deleted_on_consumption`.

**Client library** (`conclave-lib`):
- Enabled `last_resort_key_package_ext` feature in mls-rs dependency.
- Added `generate_last_resort_key_package()` using `LastResortKeyPackageExt` extension.
- Added `generate_key_packages(count)` batch method.
- Added `.key_package_lifetime(Duration::from_secs(90 * 24 * 3600))` to client builder.
- Added `upload_key_packages(entries)` batch API method.
- New tests: `test_generate_last_resort_key_package`, `test_generate_key_packages_batch`.

**TUI** (`conclave-cli/src/tui/`):
- Session restore: uploads 1 last-resort + 5 regular via `generate_initial_key_packages()` helper.
- `/keygen`: batch upload with count message.
- `/join` (welcome accept) and SSE welcome handler: upload 1 regular replacement.
- `/reset`: batch upload after identity regen.

**GUI** (`conclave-gui/src/app.rs`):
- `KeygenDone` payload changed from `Result<Vec<u8>>` to `Result<Vec<(Vec<u8>, bool)>>`.
- Session restore and login keygen tasks use `generate_initial_key_packages()`.
- `/keygen` command uses batch generation.
- `accept_welcomes` uploads 1 regular replacement via batch API.

**CLI one-shot** (`conclave-cli/src/main.rs`):
- `Keygen` command: generates 1 last-resort + 5 regular, batch upload.
- `Join` command: uploads 1 regular replacement via batch API.

## 2026-02-17: TUI SSE Reconnect — Missed Messages and Unread Counts

### What Changed

- **`conclave-cli/src/tui/mod.rs`**: The SSE `Open` handler now calls `fetch_missed_messages()` on reconnect, fetching and decrypting all messages that arrived while the client was disconnected. Previously, reconnection only updated the connection status indicator — messages sent during the offline period were lost until a new SSE event triggered a fetch for that specific room. After fetching, the screen is fully redrawn so the user sees the caught-up messages immediately. If the user is viewing a room, `last_read_seq` is updated to match `last_seen_seq` so the unread count correctly reflects only messages the user hasn't seen.

## 2026-02-17: GUI Bug Fixes

### What Changed

Four GUI bugs fixed in `conclave-gui`.

#### Offline Message Loss on Reconnect
- **`conclave-gui/src/app.rs`**: `SseUpdate::Connected` handler now fetches missed messages for all rooms on SSE reconnect. Previously it only updated the connection status without catching up on messages sent while the client was offline.

#### Tab Key Navigation on Login Screen
- **`conclave-gui/src/screen/login.rs`**: Added `FocusUsername` and `FocusPassword` message variants. Server URL input submits to focus username, username input submits to focus password, password input submits the form. Added `.id()` to all three inputs for programmatic focus.
- **`conclave-gui/src/app.rs`**: Added `TabPressed` message variant. `subscription()` now intercepts Tab key presses globally. On the login screen, Tab cycles focus between fields via `iced::widget::operation::focus_next()`. Enter on each field advances to the next field (or submits on the password field) via `iced::widget::operation::focus()`.

#### Login/Register Button Text Color
- **`conclave-gui/src/theme/text.rs`**: Added `on_primary()` text style that uses `theme.surface` (dark) color for text displayed on primary-colored backgrounds.
- **`conclave-gui/src/screen/login.rs`**: Submit button text now uses the `on_primary` text class, ensuring dark text on the light peach primary button background.

#### Commands Not Displaying Output in Rooms
- **`conclave-gui/src/app.rs`**: `show_help()`, `show_group_info()`, and `show_unread()` now use `push_system_message()` instead of pushing directly to `self.system_messages`. This routes output to the active room's message list when a room is selected, making commands like `/help`, `/info`, and `/unread` visible while viewing a room.

## 2026-02-15: Cipher Suite Upgrade to CURVE448_CHACHA

### What Changed

Upgraded the MLS cipher suite from `CURVE25519_AES128` (suite 1, 128-bit security) to `CURVE448_CHACHA` (suite 6, 256-bit security).

- **`conclave-lib/src/mls.rs`**: Changed the `CIPHERSUITE` constant from `CipherSuite::CURVE25519_AES128` to `CipherSuite::CURVE448_CHACHA`.
- **New primitives**: X448 (KEM), ChaCha20-Poly1305 (AEAD), SHA-512 (hash), Ed448 (signatures).
- **Breaking change**: Existing MLS state is incompatible. Clients must `/reset` and re-create groups.

## 2026-02-15: GUI Ctrl+Q Keybinding

### What Changed

- **`conclave-gui/src/app.rs`**: Added `Message::Quit` variant. `subscription()` now uses `iced::event::listen_with` to capture Ctrl+Q (Cmd+Q on macOS) globally, even when the text input has focus. Batched with the SSE subscription via `Subscription::batch`.

## 2026-02-15: GUI `/rooms` Empty State

### What Changed

- **`conclave-gui/src/app.rs`**: The `/rooms` command now displays system messages listing all rooms (with member lists and active indicator), or "No rooms." when the user has no rooms. Previously it produced no visible output.

## 2026-02-15: GUI SSE Auto-Reconnect

### What Changed

- **`conclave-gui/src/subscription.rs`**: `sse_stream()` now loops on error instead of terminating. On disconnect, it yields `Disconnected`, sleeps 5 seconds, then retries. Added `SseUpdate::Connecting` variant. The `reqwest::Client` is reused across reconnections.
- **`conclave-gui/src/app.rs`**: Added `SseUpdate::Connecting` handler to set `ConnectionStatus::Connecting`.
- The TUI already had 5-second auto-reconnect via `tokio::select!`.

## 2026-02-15: Warning Cleanup and Dead Code Removal

### What Changed

Addressed all compiler warnings across the workspace.

- **conclave-cli**: Removed unnecessary `mut` from 4 `api` bindings in CLI subcommand handlers.
- **conclave-gui/app.rs**: Removed unused `CommitUploaded` message variant, `CommitInfo` struct, and `RegisterInfo.username` field.
- **conclave-gui/subscription.rs**: Removed unused fields from `SseUpdate` variants — `seq`/`sender_id` from `NewMessage`, converted `Welcome` and `GroupUpdate` to fieldless variants.
- **conclave-gui/theme**: Removed unused `accent` and `unread` fields from `Theme` struct, removed unused `text::accent()`, `container::surface()`, and `container::status_bar()` functions.
- **conclave-gui/widget/message_view.rs**: Removed unused `view()` function (only `message_list()` is used).

## 2026-02-15: URL Auto-Normalization and Error Message Improvements

### What Changed

- **URL normalization**: `ApiClient::new()` now auto-prepends `https://` when the server URL has no scheme. Users can type `example.com:8443` instead of `https://example.com:8443`.
- **Error messages**: The `Error::Http` variant now walks the full reqwest error cause chain via `format_error_chain()`. Instead of the unhelpful "HTTP error: builder error", users see the complete error context (e.g., "HTTP error: error sending request: ... connection refused").

## 2026-02-15: Login Rework — Server as Parameter

### What Changed

The server URL is no longer a configuration file setting. Instead, it is specified during login and registration as a command parameter.

- **Command format**: `/login <server> <username> <password>` and `/register <server> <username> <password>`.
- **CLI**: `conclave-cli login -s <server> -u <user> -p <pass>`.
- **Session persistence**: `SessionState` now includes `server_url`. After login, the server URL is saved so subsequent commands and session restoration use it automatically.
- **`ClientConfig`**: `server_url` field removed. Only `data_dir` and `accept_invalid_certs` remain.
- **TUI**: `ApiClient` is created with an empty URL on startup (if no session) and replaced with a properly configured one on `/login`.
- **GUI**: `Conclave` struct has a `server_url: Option<String>` field used by all async operations. The login screen pre-fills from the saved session URL.
- **SSE subscription**: Now passes `accept_invalid_certs` to the reqwest client builder (previously ignored).

## 2026-02-15: TLS Support

### What Was Built

Added optional native TLS support to the server and TLS certificate validation to clients.

#### Server

- Added `tls_cert_path` and `tls_key_path` fields to `ServerConfig`. When both are set, the server uses `axum-server` with `rustls` to serve HTTPS. When omitted, it falls back to plain HTTP (for use behind a reverse proxy).
- New dependencies: `axum-server` (0.8, with `tls-rustls` feature), `rustls-pemfile` (2).

#### Client

- Added `accept_invalid_certs` field to `ClientConfig` (default: `false`). When `false`, `reqwest` validates the server's TLS certificate normally. When `true`, certificate validation is skipped (for development with self-signed certs).
- `ApiClient::new()` now accepts the `accept_invalid_certs` flag and configures the `reqwest::Client` accordingly.
- Both CLI and GUI clients pass the config flag through to `ApiClient`.

## 2026-02-15: GUI Client (iced)

### What Was Built

Full graphical client using iced 0.14 with Elm-style architecture (model → update → view).

#### Architecture

- **`conclave-gui` crate**: New workspace member with `conclave-lib` as shared dependency.
- **Screens**: Login (centered card with server URL, username, password, login/register toggle) and Dashboard (three-panel layout: sidebar with room list + unread counts, scrollable message area, chat input).
- **Theme**: Custom dark theme (Ferra-inspired palette) implementing `iced::theme::Base` with per-widget `Catalog` styles for buttons, containers, text, text inputs, and scrollables.
- **Subscriptions**: SSE event stream via `iced::Subscription::run_with()` keyed by auth token, plus a 1-second tick timer for connection status.
- **Async**: All API calls via `Task::perform()`. MLS crypto (sync) wrapped in `tokio::task::spawn_blocking`.
- **Commands**: All TUI `/` commands supported in the GUI text input.

#### Bug Fixes

- **Wrong server URL**: `LoginInfo` now carries the server URL from the login form so `ApiClient` connects to the correct server.
- **Room list not refreshing**: Group creation, invite, and kick operations now trigger automatic room list reload.
- **"group mapping not found"**: `create_group` now returns a `GroupCreated` message that updates `self.group_mapping` before switching to the new room.

## 2026-02-15: Shared Library Extraction (`conclave-lib`)

### What Was Built

Extracted reusable client logic from `conclave-cli` into a new `conclave-lib` library crate so both the CLI/TUI and GUI can share it.

#### Modules Moved

| Module | From | To |
|--------|------|----|
| `api.rs` | `conclave-cli/src/` | `conclave-lib/src/api.rs` |
| `mls.rs` | `conclave-cli/src/` | `conclave-lib/src/mls.rs` |
| `config.rs` | `conclave-cli/src/` | `conclave-lib/src/config.rs` |
| `error.rs` | `conclave-cli/src/` | `conclave-lib/src/error.rs` (removed `Terminal` variant) |
| `Room`, `DisplayMessage`, `ConnectionStatus` | `conclave-cli/src/tui/state.rs` | `conclave-lib/src/state.rs` |
| `MessageStore` | `conclave-cli/src/tui/store.rs` | `conclave-lib/src/store.rs` |
| `Command` enum + `parse()` | `conclave-cli/src/tui/commands.rs` | `conclave-lib/src/command.rs` |

#### Crate Rename

`conclave-client` was renamed to `conclave-cli` to clarify that it is a CLI/TUI application binary, not a library. All workspace references, docs, and AGENT.md were updated.

## 2026-02-15: Initial Implementation

### What Was Built

Complete server and client implementation from scratch:

- **conclave-proto**: Protobuf schema with 20+ message types, prost code generation.
- **conclave-server**: Full axum HTTP server with SQLite storage, Argon2id auth, opaque token sessions, 11 API endpoints, SSE real-time push.
- **conclave-cli**: CLI/TUI client with MLS E2EE (mls-rs + OpenSSL), SQLite-persisted MLS state, one-shot commands, and interactive TUI.

End-to-end encryption verified: two users can register, create an MLS group, exchange encrypted messages, and decrypt them.

### Known Issues and Limitations

#### MLS Epoch Handling

When a user fetches messages, the list includes MLS commit messages (e.g., the initial commit from group creation at seq 1). For users who joined via a welcome message, these commits are for an epoch they've already processed. The client currently handles this by catching decryption errors and returning `None` (silently skipping). A cleaner solution would be to either:

1. Tag messages in the database with a type field (commit vs. application) so clients can skip commits they don't need.
2. Store commits and application messages in separate tables/endpoints.
3. Track each client's last processed sequence number server-side and only return newer messages.

#### Own Messages Not Visible

Due to MLS's ratcheting, a sender cannot decrypt their own messages from the server. The encrypted ciphertext was produced with a key that has since been ratcheted forward. In a production client, sent messages should be stored locally in plaintext alongside their sequence number so they can be displayed in the message history. The current implementation does not do this.

#### ~~Single Key Package~~ (Resolved)

Each user now pre-publishes multiple key packages (1 last-resort + 5 regular). Regular packages are consumed FIFO; the last-resort package is never deleted on consumption, ensuring the user is always reachable. Server enforces a cap of 10 regular packages. Clients auto-replenish after consumption and upload a full batch on login/keygen/reset. See the "RFC 9420-Compliant Key Package Lifecycle" entry above.

#### No Message Ordering Guarantees

The server assigns sequence numbers per group, but there is no mechanism to ensure clients process messages in order. Out-of-order MLS message processing is enabled via the `rfc_compliant` feature (which includes `out_of_order`), but the client fetches all messages from seq 0 each time, which may cause issues if messages are processed multiple times. A cursor/watermark per user per group would fix this.

#### BasicCredential Trust Model

The MLS identity uses `BasicCredential` with `BasicIdentityProvider`, which accepts any credential without validation. This is fine for a closed community where the server is trusted to enforce usernames, but provides no cryptographic identity binding. Upgrading to X.509 credentials would add stronger identity assurance.

#### ~~No TLS~~ (Resolved)

The server now supports optional native TLS via `axum-server` with `rustls`. Set `tls_cert_path` and `tls_key_path` in the server config. Alternatively, run behind a TLS-terminating reverse proxy with plain HTTP mode. Clients validate TLS certificates by default (`accept_invalid_certs = false`).

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

## 2026-02-15: IRC-Style TUI Redesign

### What Changed

Replaced the blocking rustyline REPL (`repl.rs`) with an IRC-style interactive TUI using crossterm for raw terminal mode and reqwest-eventsource for real-time SSE message delivery.

### New Architecture

The TUI module (`src/tui/`) has 6 files:

- **`mod.rs`** — Main event loop using `tokio::select!` over crossterm key events, SSE events, and a reconnection timer. Manages raw mode setup/teardown with alternate screen.
- **`state.rs`** — `AppState` (rooms, active room, message history, group mapping, connection status), `Room`, `DisplayMessage`, `ConnectionStatus`.
- **`input.rs`** — `InputLine` line editor with cursor movement, command history (up/down), and standard editing keys.
- **`render.rs`** — Terminal drawing: message area (scrollable), reverse-video status line, input line with room prefix. ANSI nick coloring via username hash.
- **`commands.rs`** — IRC-style command parsing and execution. Lines starting with `/` are commands; plain text sends to the active room.
- **`events.rs`** — SSE event decoding (hex-encoded protobuf) and handling: new message fetch + MLS decrypt, welcome processing, group updates.

### Key UX Changes

- **Room context**: Users `/join` a room and type messages directly (no `/send <group_id>` needed)
- **Real-time messages**: SSE pushes `NewMessageEvent` notifications; client fetches and decrypts new messages automatically
- **Status line**: Shows connection status, active room with member count, and username
- **IRC commands**: `/create`, `/join`, `/part`, `/rooms`, `/who`, `/invite`, `/msg`, `/help`, `/quit`
- **Sent messages displayed locally**: Sender sees their own messages immediately without waiting for SSE echo

### Dependency Changes

- **Removed**: `rustyline`
- **Added**: `crossterm` (event-stream), `reqwest-eventsource`, `futures-util`, `chrono`

### MLS Threading

MLS operations are sync (mls-rs compiled in sync mode). In the async TUI event loop, MLS decrypt/encrypt operations from SSE event handling use `tokio::task::spawn_blocking` with a fresh `MlsManager` constructed inside the closure. Command-initiated MLS operations (send, create, join) run inline since the event loop is blocked during command execution anyway.

### SSE Reconnection

The event loop includes a 5-second reconnection timer. When the SSE connection drops, the client automatically reconnects. Message continuity is maintained via `Room.last_seen_seq` — on reconnect, the client fetches only messages after the last seen sequence number.

### What to Build Next

1. ~~**Multiple key packages**~~: Resolved — see "RFC 9420-Compliant Key Package Lifecycle" entry.
2. **Message types in DB**: Add a `message_type` column (commit, application, proposal) to help clients filter.
3. **X.509 credentials**: Upgrade from BasicCredential for stronger identity assurance.
4. **Tab completion**: Command and room name auto-completion in the TUI.

## 2026-02-15: Comprehensive MLS Feature Implementation

### What Was Built

Implemented the full set of practical MLS group management features across server and client.

#### Member Removal (`/kick`)
- Server: `remove_group_member()` DB method, `POST /groups/{id}/remove` endpoint with SSE `MemberRemovedEvent` notification to both remaining and removed members.
- Client MLS: `remove_member()` using `CommitBuilder::remove_member()`, `find_member_index()` to look up members by username in the MLS roster.
- Client TUI: `/kick <username>` command.

#### Leave Group (`/leave`)
- Server: `POST /groups/{id}/leave` endpoint, removes user from DB and notifies remaining members via SSE.
- Client: `/leave` command deletes local MLS group state and removes from local room list. `/part` now only switches the view without leaving.

#### Key Rotation (`/rotate`)
- Client MLS: `rotate_keys()` performs an empty commit to advance the epoch for forward secrecy / post-compromise security.
- Client TUI: `/rotate` command uploads the commit to the server.

#### Account Reset (`/reset`)
- Server: `POST /reset-account` clears key packages, `GET /groups/{id}/group-info` serves stored GroupInfo, `POST /groups/{id}/external-join` processes external commits.
- Client MLS: `external_rejoin_group()` uses `Client::external_commit_builder().with_removal()` to rejoin with a new identity. `wipe_local_state()` deletes identity and state files.
- Client TUI: `/reset` command wipes local MLS state, regenerates identity, uploads new key package, and external-commits to rejoin all groups.

#### Group Info Display (`/info`)
- Client MLS: `group_info_details()` returns epoch, cipher suite, member count, own leaf index, and full member list.
- Client TUI: `/info` command displays all MLS group details.

#### Improved Commit Processing
- `decrypt_message()` now returns `DecryptedMessage` enum (`Application`, `Commit`, `None`) instead of `Option<Vec<u8>>`.
- `CommitInfo` struct extracts roster changes (members added/removed, self-removal detection) from `CommitEffect` and applied proposals.
- TUI displays system messages: "X was added", "Y was removed", "Z updated their keys".
- `MemberRemovedEvent` SSE handling cleans up local state when removed by another member.

#### Server Changes
- New `group_infos` table for storing GroupInfo blobs (needed for external commits).
- `upload_commit` handler now stores GroupInfo via `store_group_info()`.
- `group_info_message_allowing_ext_commit(true)` used instead of `group_info_message(true)` so GroupInfo supports external commits.
- 5 new API endpoints: remove member, leave group, get group info, external join, reset account.

#### Proto Changes
- New message types: `RemoveMemberRequest`, `LeaveGroupRequest`, `GetGroupInfoResponse`, `ResetAccountResponse`, `ExternalJoinRequest`, `MemberRemovedEvent`.
- `ServerEvent` oneof extended with `member_removed` variant.

### Known Issues Resolved

- **No Member Removal**: Now fully implemented via `/kick` and `/leave`.
- **Unread indicators**: `/unread` command checks all rooms for new messages.

### Remaining Limitations

- **Leave group MLS-level**: `/leave` removes the user from the server DB and deletes local state, but the stale MLS leaf remains in the group tree until another member commits. This is acceptable — the user is fully removed from the server's perspective.
- **External commit after reset**: Requires that GroupInfo was stored (via a prior commit upload). If no commit has been uploaded for a group, the reset cannot rejoin it.

## 2026-02-15: Comprehensive Test Suite

### What Was Built

Added 189 tests across 8 test suites with both positive and negative cases for all features. Zero bugs found — all existing code behaved correctly.

### Restructuring for Testability

- **Server `lib.rs`**: Created `crates/conclave-server/src/lib.rs` re-exporting all modules (`pub mod api, auth, config, db, error, state`). `main.rs` changed from `mod` declarations to `use conclave_server::*`. This enables integration tests in `tests/` to access server internals.
- **Ungated `open_in_memory()`**: Removed `#[cfg(test)]` from `Database::open_in_memory()` so integration tests can create in-memory databases.
- **Dev-dependencies**: Added `tower` (util), `http-body-util`, `tokio` (macros) for server; `tempfile` for client.

### Test Breakdown

| Suite | File | Tests | Coverage |
|-------|------|-------|----------|
| Server DB | `db.rs` (inline) | 35 | All DB methods: users, sessions, key packages (accumulate, cap, last-resort), groups, members, messages, welcomes, group info |
| Server Auth | `auth.rs` (inline) | 5 | Password hashing/verification, token generation, invalid hash handling |
| Server API | `tests/api_tests.rs` | 48 | All 18 HTTP endpoints via `tower::oneshot()`: registration, login, logout, key packages (batch upload, ordered consumption, last-resort), groups, messages, invites, member removal, leave, group info, external join, reset, commits, welcomes |
| Client MLS | `mls.rs` (inline) | 25 | Key package generation (regular, last-resort, batch), group lifecycle, encrypt/decrypt roundtrip, commit processing, member removal, key rotation, external rejoin, identity persistence, state cleanup |
| Client Commands | `commands.rs` (inline) | 33 | All 21 command variants parsed, missing args, unknown commands, edge cases |
| Client InputLine | `input.rs` (inline) | 19 | Cursor movement, editing, history navigation, credential exclusion from history |
| Client AppState | `state.rs` (inline) | 14 | Room management, message routing, room lookup (exact/prefix/case-insensitive) |
| Client MessageStore | `store.rs` (inline) | 10 | SQLite message persistence, sequence tracking, group isolation |

### Testing Patterns

- **Server API tests**: Use `tower::ServiceExt::oneshot()` with in-memory SQLite — no TCP listener needed. Each test creates a fresh `Router` via `setup()`. Protobuf encoding/decoding for request/response bodies.
- **MLS crypto tests**: Use `tempfile::TempDir` for isolated MLS state directories. Real cryptographic operations (no mocking) — tests verify actual encrypt/decrypt roundtrips, key rotation epoch advancement, and external commit rejoins.
- **Negative tests**: Every critical feature has failure-path coverage (invalid inputs return errors, non-members get 401, missing resources get 404, etc.).
