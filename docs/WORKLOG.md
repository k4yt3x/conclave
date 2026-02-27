# Conclave Work Log

## 2026-02-27: Remove Old Password Requirement from /passwd

Removed the `current_password` field from the `/passwd` command and `POST /api/v1/change-password` endpoint. The old password was previously required for verification, but since the user is already authenticated via session token (`AuthUser` extractor), re-verifying the old password was redundant. This also fixes a usability bug where passwords containing spaces could not be entered as the current password argument due to whitespace-based argument splitting.

### Changes
- **Proto**: Removed `current_password` field from `ChangePasswordRequest`; only `new_password` remains.
- **Server handler**: Removed `get_password_hash()` lookup and `verify_password()` check from `change_password()` handler in `api/auth.rs`. The handler now only validates the new password, hashes it, and updates the DB.
- **Client API**: Changed `change_password()` signature from `(current_password, new_password)` to `(new_password)`.
- **Command**: Changed `Passwd` variant from two fields (`current_password`, `new_password`) to one field (`new_password`). Updated `CommandSpec` usage to `/passwd <new_password>`. Changed parser from `splitn(3, ' ')` to `splitn(2, ' ')` so the entire argument (including spaces) is the new password.
- **CLI/GUI**: Updated dispatch to match the single-field `Passwd` variant.
- **Tests**: Removed `test_change_password_wrong_current` (no longer applicable). Updated remaining 6 server API tests and 3 client command parser tests to remove `current_password`.

## 2026-02-27: Add Change Password Feature

Added a `/passwd` command and `POST /api/v1/change-password` endpoint allowing authenticated users to change their account password. Existing sessions remain valid after a password change. Extracted inline password validation from the register handler into a reusable `validate_password()` function in `validation.rs`.

### Changes
- **Proto**: Added `ChangePasswordRequest` (with `new_password`) and `ChangePasswordResponse` messages.
- **Server validation**: Added `validate_password()` to `validation.rs` (min 8 chars). Refactored `register` handler to use it instead of inline check.
- **Server DB**: Added `get_password_hash()` and `update_user_password()` methods to `db/users.rs`.
- **Server handler**: Added `change_password()` handler in `api/auth.rs` — validates new password, hashes with Argon2id, updates DB. Logged at `info` level.
- **Server router**: Added `POST /api/v1/change-password` route in the Authentication group.
- **Client API**: Added `change_password()` method to `ApiClient`.
- **Command**: Added `CommandSpec` for `passwd` in Account category, `Passwd` variant to `Command` enum, and `parse_command_args` branch using `splitn(2, ' ')` so the new password can contain spaces.
- **CLI**: Added `Command::Passwd` to `execute_profile` dispatch in `tui/commands.rs`.
- **GUI**: Added `Command::Passwd` handler using `Message::CommandResult` in `app/commands.rs`.
- **Tests**: Added 6 server API tests (success, old password invalidated, short new password, unauthenticated, empty new password, session stays valid). Added 3 client command parser tests (success, spaces in password, missing args).

## 2026-02-27: Remove Embedded Names from StoredMessage, Add User Lookup by ID

Continued the ID-first convention enforcement. Removed `sender_username` and `sender_alias` fields from `StoredMessage` protobuf — the server no longer JOINs the users table when fetching messages. Clients now resolve sender display names from their local member cache (populated by `ListGroupsResponse`). Added `inviter_id` field to `PendingInvite` so both the ID and display name are available. Added `GET /api/v1/users/by-id/{user_id}` endpoint for client-side ID→name resolution when the local cache has a miss (e.g., for users who have left the group).

### Changes
- **Proto**: Removed `sender_username` (field 3) and `sender_alias` (field 6) from `StoredMessage`. Added `inviter_id` (field 8) to `PendingInvite`.
- **Server**: Simplified `db/messages.rs` `get_messages()` query to remove JOIN. Added `get_user_by_id` handler and `/api/v1/users/by-id/{user_id}` route. Populated `inviter_id` in invite list responses.
- **Client**: `operations/messaging.rs` now resolves sender display name from member cache via `resolve_user_display_name()` instead of using server-provided username. Added `get_user_by_id()` API method.
- **Tests**: Added `test_get_user_by_id_success` and `test_get_user_by_id_not_found`. Added `inviter_id` assertion to existing invite test.

## 2026-02-27: Enforce user_id/group_id as Primary Identifiers Everywhere

Refactored all request messages (client-to-server) and SSE events to use integer IDs (`user_id`/`group_id`) instead of username strings. Response/info messages (like `GroupMember`, `StoredMessage`, `PendingInvite`) retain both ID and username for display convenience. Username resolution now happens at the UI boundary (CLI/GUI command handlers), not in operations or API layers.

### Protobuf changes (`conclave.proto`)
- `InviteToGroupRequest.usernames` → `repeated int64 user_ids`
- `InviteToGroupResponse.member_key_packages` → `map<int64, bytes>`
- `EscrowInviteRequest.invitee_username` → `int64 invitee_id`
- `PromoteMemberRequest.username` → `int64 user_id`
- `DemoteMemberRequest.username` → `int64 user_id`
- `RemoveMemberRequest.username` → `int64 user_id`
- `MemberRemovedEvent.removed_username` → `int64 removed_user_id`
- `IdentityResetEvent.username` → `int64 user_id`
- `InviteReceivedEvent.inviter_username` → `int64 inviter_id`
- `InviteDeclinedEvent.declined_username` → `int64 declined_user_id`

### Server changes
- **`api/members.rs`**: `invite_to_group` accepts `user_ids`, verifies via `get_user_by_id()`. `remove_group_member`, `promote_member`, `demote_member` accept `user_id` directly. `leave_group` broadcasts `removed_user_id`.
- **`api/invites.rs`**: `escrow_invite` accepts `invitee_id`. `cancel_invite`/`decline_invite` broadcast `declined_user_id`. `InviteReceivedEvent` uses `inviter_id`.
- **`api/external.rs`**: `external_join` broadcasts `IdentityResetEvent` with `user_id`.

### Client library changes
- **`mls.rs`**: All `HashMap` keys changed from `String` to `i64` in `GroupCreationResult`, `InviteResult`, `build_commit_with_members`, `create_group`, `invite_to_group`. Internal `kp_ref_to_username` → `kp_ref_to_user_id`. All MLS tests updated.
- **`api.rs`**: `invite_to_group`, `escrow_invite`, `promote_member`, `demote_member`, `remove_member` accept IDs instead of strings.
- **`operations/groups.rs`**: `invite_members` takes `Vec<i64>`, returns `Vec<i64>`. `kick_member` removed `target_username` parameter. `cancel_invite` takes `user_id` instead of username.
- **`operations.rs`**: `SseEvent` enum variants updated to use `removed_user_id`, `user_id`, `inviter_id`, `declined_user_id`. `decode_sse_event()` updated.

### CLI changes
- **`tui/commands.rs`**: `/invite`, `/kick`, `/promote`, `/demote`, `/uninvite` resolve usernames → user_ids at the UI boundary before calling operations.
- **`tui/events.rs`**: SSE handlers use user_id for self-detection and resolve display names from local member list.
- **`main.rs`**: One-shot `Invite` command resolves usernames → user_ids.

### GUI changes
- **`subscription.rs`**: `SseUpdate` enum variants updated to use IDs.
- **`app/sse.rs`**: SSE handlers use user_id for self-detection and resolve display names from local member list.
- **`app/rooms.rs`**: `invite_members`, `kick_member`, `promote_member`, `demote_member`, `cancel_invite` resolve usernames → user_ids at the UI boundary.

### Test updates
- **`api_tests.rs`**: All 158 tests updated to use `user_id`/`user_ids` in request construction and SSE assertions.
- **`protocol_flow_tests.rs`**: All 9 tests updated. `invite_members` and `escrow_and_accept_invite` helpers take IDs instead of strings.

## 2026-02-26: Add `/invited` and `/uninvite` Commands + Fix `/register` Description

### `/register` description fix
- Updated description from "Register a new account" to "Register a new account and login" in `command.rs` and CLI `main.rs`, matching actual behavior (register + auto-login).

### New commands: `/invited` and `/uninvite`
Added two admin commands for managing pending invites on the active room:
- `/invited` — List pending invites for the active room (admin-only)
- `/uninvite <username>` — Cancel a pending invite (admin-only)

### Protobuf changes (`conclave.proto`)
- Added `invitee_id` field (7) to `PendingInvite` message
- Added `ListGroupPendingInvitesResponse`, `CancelInviteRequest`, `CancelInviteResponse` messages
- Added `InviteCancelledEvent` message and `invite_cancelled` (8) to `ServerEvent.oneof`

### Server changes
- **Database** (`db/invites.rs`): Added `list_pending_invites_for_group()` and `get_pending_invite_by_group_and_invitee()` methods
- **API** (`api/invites.rs`): Added `list_group_pending_invites` (GET `/api/v1/groups/{group_id}/invites`) and `cancel_invite` (POST `/api/v1/groups/{group_id}/cancel-invite`) handlers. Both require admin role. Cancel sends `InviteCancelledEvent` to invitee and `InviteDeclinedEvent` to inviter (for phantom MLS leaf cleanup)
- **Routes** (`api.rs`): Registered both new endpoints
- **Existing handler**: Populated new `invitee_id` field in `list_pending_invites` response
- **Tests**: Added 7 tests — `test_list_group_pending_invites_empty`, `test_list_group_pending_invites_success`, `test_list_group_pending_invites_not_admin`, `test_cancel_invite_success`, `test_cancel_invite_not_found`, `test_cancel_invite_not_admin`, `test_cancel_invite_nonexistent_user`

### Client changes
- **API** (`api.rs`): Added `list_group_pending_invites()`, `cancel_invite()`, and `get_user_by_username()` methods
- **SSE** (`operations.rs`): Added `SseEvent::InviteCancelled` variant and decoding
- **Operations** (`operations/groups.rs`): Added `list_group_pending_invites()` and `cancel_invite()` (resolves username → user_id internally)
- **Command parser** (`command.rs`): Added `Invited` and `Uninvite` to COMMANDS array (Members category), Command enum, and parser. Added 3 tests

### CLI TUI changes
- **Commands** (`tui/commands.rs`): Added handlers for `Invited` (shows invitee + inviter names) and `Uninvite` (calls cancel_invite)
- **SSE** (`tui/events.rs`): Handles `InviteCancelled` event with system message

### GUI changes
- **Commands** (`app/commands.rs`): Routes `Invited` and `Uninvite` commands
- **Room operations** (`app/rooms.rs`): Added `list_group_invites()` and `cancel_invite()` methods
- **SSE** (`app/sse.rs`, `subscription.rs`): Added `InviteCancelled` SSE variant and handler

All 507 tests pass. Release build clean (no warnings).

## 2026-02-24: Code Style Review — Refinements

Applied 8 code style improvements across the workspace. No behavioral changes — all 497 tests pass.

### Error handling improvements
- **`store.rs`**: Replaced `let _ = std::fs::set_permissions(...)` with `if let Err` + `tracing::warn!`
- **`tui.rs`**: Added `tracing::warn!` logging for terminal cleanup errors during shutdown (previously `let _ =`)
- **`subscription.rs`**: Replaced `.unwrap_or_default()` on fallible `reqwest::Client::builder().build()` with `.unwrap_or_else()` that logs the error
- **`main.rs`**: Added empty-string filtering on CLI `/invite` member input to reject `alice,,bob` cleanly

### Test quality improvements
- Replaced ~35 `panic!("wrong variant")` / `panic!("expected ...")` patterns across `command.rs`, `mls.rs`, and `operations.rs` with `let ... else { panic!("expected X variant") }` pattern for clearer test failures

### Code structure improvements
- **`operations.rs`**: Extracted `map_join_error()` helper for `tokio::task::spawn_blocking` join error mapping, replacing 9 identical `.map_err(|e| Error::Other(format!("task join error: {e}")))` closures across `messaging.rs`, `groups.rs`, and `account.rs`
- **`db/groups.rs`**: Replaced `(i64, String, Option<String>, String)` tuple return from `get_group_members()` with named `GroupMemberRow` struct, removing `#[allow(clippy::type_complexity)]`. Updated all call sites in `api.rs`, `api/groups.rs`, `api/members.rs`, `api/external.rs`, and test code
- **`render.rs`**: Removed unused `_msg` parameter from `render_new_message()` and updated its call site

### Deferred items
- Added 3 architectural cleanup items to `TODO.md`: dashboard `view()` parameter count, room display duplication, GUI startup task ordering

## 2026-02-24: Codebase Organization — Module Splits and Deduplication

Comprehensive refactoring to split large files into domain modules, extract duplicated code, and consolidate cross-crate patterns. No behavioral changes — all 497 tests pass.

### Server: Split `db.rs` (2,254 lines) into domain modules

Split into `db.rs` (module root with schema/init) + 6 sub-modules, each adding methods via `impl Database` blocks:

- `db/users.rs` — user CRUD (create, get by id/username, update alias)
- `db/sessions.rs` — session management (create, validate, delete, cleanup)
- `db/key_packages.rs` — key package operations (store, consume, count, delete)
- `db/groups.rs` — group CRUD, member/admin management (create, list, update, member add/remove, admin promote/demote)
- `db/messages.rs` — message storage and retrieval, commit processing, group info
- `db/invites.rs` — invite lifecycle (create escrow, list, accept, decline, cleanup) and welcome operations

### Server: Split `api.rs` (1,369 lines) into handler modules

Split into `api.rs` (router + helpers) + 8 sub-modules:

- `api/auth.rs` — register, login, logout, me, update_profile, get_user_by_username, reset_account
- `api/key_packages.rs` — upload_key_package, get_key_package
- `api/groups.rs` — create_group, list_groups, update_group, get_group_info
- `api/messages.rs` — send_message, get_messages, upload_commit
- `api/members.rs` — invite_to_group, escrow_invite, remove_group_member, leave_group, promote/demote_member, list_admins
- `api/invites.rs` — list_pending_invites, accept_invite, decline_invite
- `api/welcomes.rs` — list_pending_welcomes, accept_welcome
- `api/external.rs` — external_join

### Server: Extract SSE broadcast helper

Extracted `notify_group_members(state, group_id, exclude_user_id, event)` helper in `api.rs`, replacing ~11 instances of the duplicated broadcast pattern.

### Server: Extract validation module

Moved `validate_username()`, `validate_group_name()`, `validate_alias()` and their constants from `db.rs` to a new `validation.rs` module.

### Client lib: Split `operations.rs` (1,180 lines) into sub-modules

Split into `operations.rs` (types, SSE decoding, load_rooms, tests) + 3 sub-modules:

- `operations/messaging.rs` — fetch_and_decrypt, send_message, rotate_keys, process_commit_info (extracted helper)
- `operations/groups.rs` — create_group, invite_members, accept/decline invites, kick_member, leave_group, accept_welcomes
- `operations/account.rs` — reset_account, rejoin_groups_via_external_commit (extracted helper), initialize_mls_and_upload_key_packages (new consolidated function)

### Client lib: Extract MLS commit-building helper

Extracted `build_commit_with_welcomes()` in `mls.rs` to deduplicate the commit+welcome construction logic shared between `create_group()` and `invite_to_group()`.

### Client lib: Add `ProcessedMessage::system()` constructor

Reduced 6 instances of boilerplate system message construction to use a shared constructor.

### Client lib: Consolidate key package init/upload

Created `initialize_mls_and_upload_key_packages(api, data_dir, user_id)` in `operations/account.rs`, consolidating the duplicated flow from CLI one-shot (register + login), TUI (register + login), and GUI (register).

### CLI: Refactor `handle_key_event()`

Split 287-line function into: special keys (Ctrl+C, Enter, PageUp/Down) + input-editing keys with a single shared render call. Extracted `handle_enter()` async function for Enter key command dispatch.

### CLI: Merge Msg/Message command duplication

Extracted `send_to_group()` helper in `commands.rs`, eliminating ~40 lines of duplicated code between the `Msg` and `Message` command handlers.

### Files Created

- `crates/conclave-server/src/validation.rs`
- `crates/conclave-server/src/db/users.rs`
- `crates/conclave-server/src/db/sessions.rs`
- `crates/conclave-server/src/db/key_packages.rs`
- `crates/conclave-server/src/db/groups.rs`
- `crates/conclave-server/src/db/messages.rs`
- `crates/conclave-server/src/db/invites.rs`
- `crates/conclave-server/src/api/auth.rs`
- `crates/conclave-server/src/api/key_packages.rs`
- `crates/conclave-server/src/api/groups.rs`
- `crates/conclave-server/src/api/messages.rs`
- `crates/conclave-server/src/api/members.rs`
- `crates/conclave-server/src/api/invites.rs`
- `crates/conclave-server/src/api/welcomes.rs`
- `crates/conclave-server/src/api/external.rs`
- `crates/conclave-client/src/operations/messaging.rs`
- `crates/conclave-client/src/operations/groups.rs`
- `crates/conclave-client/src/operations/account.rs`

### Files Modified

- `crates/conclave-server/src/lib.rs` — added `pub mod validation`
- `crates/conclave-server/src/db.rs` — module root with schema/init only
- `crates/conclave-server/src/api.rs` — module root with router + helpers only
- `crates/conclave-client/src/operations.rs` — module root with types + SSE decoding
- `crates/conclave-client/src/mls.rs` — extracted `build_commit_with_welcomes()`
- `crates/conclave-cli/src/main.rs` — use consolidated key package init
- `crates/conclave-cli/src/tui.rs` — refactored handle_key_event, extracted handle_enter
- `crates/conclave-cli/src/tui/commands.rs` — extracted send_to_group, use consolidated key package init
- `crates/conclave-gui/src/app/login.rs` — use consolidated key package init

## 2026-02-23: Codebase Decluttering — Split Large Files

Split the two largest files in the codebase for better navigability without changing any behavior.

### Part 1: Split `app.rs` (GUI)

Split the 2,278-line `app.rs` (55% of the GUI crate) into `app.rs` + 4 sub-modules using the `app.rs` + `app/` directory pattern (same as `theme.rs`/`theme/` and `screen.rs`/`screen/`). Each sub-module has its own `impl Conclave` block. Struct fields changed to `pub(crate)` for cross-module access.

- `app.rs` (~530 lines) — types, core loop (`new`, `update`, `view`, `subscription`), helpers
- `app/login.rs` (~260 lines) — `handle_login_message`, `handle_login_result`, `handle_register_result`, `handle_keygen_done`, `perform_logout`
- `app/commands.rs` (~310 lines) — `handle_dashboard_message`, `handle_input_text`
- `app/sse.rs` (~160 lines) — `handle_sse_event`
- `app/rooms.rs` (~640 lines) — room state handlers, message sending, group operations, welcomes

### Part 2: Extract Command Groups in `commands.rs` (TUI)

Extracted the 936-line `execute()` match into 6 category-specific async helper functions in the same file. The main `execute()` is now a thin dispatcher (~30 lines).

- `execute_auth()` — Register, Login, Logout
- `execute_room()` — Create, List, Join, Close, Part, Unread, Info
- `execute_member()` — Invite, Kick, Promote, Demote, Admins, Who
- `execute_invite()` — Invites, Accept, Decline
- `execute_messaging()` — Msg, Message, Rotate
- `execute_profile()` — Nick, Topic, Whois, Help, Quit, Reset

### Files Modified/Created

- Modified: `crates/conclave-gui/src/app.rs`
- Created: `crates/conclave-gui/src/app/login.rs`
- Created: `crates/conclave-gui/src/app/commands.rs`
- Created: `crates/conclave-gui/src/app/sse.rs`
- Created: `crates/conclave-gui/src/app/rooms.rs`
- Modified: `crates/conclave-cli/src/tui/commands.rs`
- Modified: `docs/WORKLOG.md`

## 2026-02-23: Simplify Group Creation to Creator-Only

Removed multi-member group creation. `/create <name>` now creates a group with only the creator as a member. All members must be added via `/invite`, which goes through the two-phase escrow system requiring acceptance. This eliminates the consent bypass where initial members were added without their approval.

### Changes

- **Proto**: Removed `member_usernames` from `CreateGroupRequest`, `member_key_packages` from `CreateGroupResponse`, `welcome_messages` from `UploadCommitRequest`. Field numbers reserved with comments to prevent reuse.
- **Server API**: Simplified `create_group` (removed key-package consumption loop). Simplified `upload_commit` (removed welcome handling entirely — welcomes are now only delivered through escrow invites). Simplified commit notification to exclude only the sender.
- **Server DB**: Removed `welcome_messages` parameter and loop from `process_commit`. Removed `NewMember` struct and `new_members` field from `CommitResult`.
- **Server main**: Fixed invite cleanup to use `config.invite_ttl_seconds` instead of hardcoded `7 * 24 * 3600`.
- **Client API**: Removed `member_usernames` from `create_group()`, removed `welcome_messages` from `upload_commit()`.
- **Client operations**: Removed `members` parameter from `create_group()`. Updated `rotate_keys()` to match new `upload_commit` signature.
- **Commands**: Changed `Command::Create` from `{ name, members }` to `{ name }`. Updated parser to accept exactly one argument. Updated help text to `/create <name>`.
- **CLI**: Updated TUI handler and one-shot `CreateGroup` subcommand (removed `--members`).
- **GUI**: Updated `create_group()` method and `Command::Create` match arm.
- **Tests**: Replaced `add_member_to_group` (commit+welcome) with `add_member_via_escrow` (escrow invite flow) in api_tests. Removed 4 obsolete tests. Rewrote all 7 protocol flow tests to use escrow invite pattern. Updated all `create_group_for` callers. Total: ~491 tests.
- **Docs**: Updated SPEC.md endpoint descriptions, command syntax, group creation flow diagram, security section, and protobuf schema description.

### Files Modified

- `proto/conclave/v1/conclave.proto`
- `crates/conclave-server/src/api.rs`
- `crates/conclave-server/src/db.rs`
- `crates/conclave-server/src/main.rs`
- `crates/conclave-server/tests/api_tests.rs`
- `crates/conclave-server/tests/protocol_flow_tests.rs`
- `crates/conclave-client/src/api.rs`
- `crates/conclave-client/src/operations.rs`
- `crates/conclave-client/src/command.rs`
- `crates/conclave-cli/src/tui/commands.rs`
- `crates/conclave-cli/src/main.rs`
- `crates/conclave-gui/src/app.rs`
- `docs/SPEC.md`
- `docs/WORKLOG.md`
- `AGENTS.md`

## 2026-02-22: Two-Phase Invite System with Escrow

Introduced a consent-based invite system for post-creation member additions. Previously, inviting a user immediately added them to the group with no option to decline — creating a DDoS vector where a malicious admin could spam-invite users into hundreds of groups. Now, the admin pre-builds the MLS commit+welcome and uploads it to escrow. The target receives an `InviteReceivedEvent` and can accept or decline. Group creation remains immediate (initial members are added directly).

### Changes

- **Proto**: Added `EscrowInviteRequest/Response`, `PendingInvite`, `ListPendingInvitesResponse`, `AcceptInviteResponse`, `DeclineInviteResponse`, `InviteReceivedEvent`, `InviteDeclinedEvent` messages. Extended `ServerEvent` oneof with `invite_received` and `invite_declined`.
- **Database**: Added `pending_invites` table with `UNIQUE(group_id, invitee_id)` constraint and invitee index. Added 7 DB methods: `create_pending_invite`, `get_pending_invite`, `list_pending_invites_for_user`, `accept_pending_invite` (atomic savepoint transaction with membership pre-check), `delete_pending_invite`, `cleanup_expired_invites`, `get_group_name`. Constraint violations on duplicate invites now return proper `Conflict` errors instead of raw DB errors.
- **API**: Added 4 new endpoints: `POST /groups/{id}/escrow-invite` (admin-only), `GET /invites`, `POST /invites/{id}/accept` (invitee-only, atomic), `POST /invites/{id}/decline` (invitee-only). The `upload_commit` endpoint now rejects welcome messages for initialized groups (mls_group_id set), preventing escrow bypass. Added periodic cleanup of expired invites (7-day TTL, hourly check).
- **Client API**: Added `escrow_invite`, `list_pending_invites`, `accept_pending_invite`, `decline_pending_invite` methods.
- **Operations**: Rewrote `invite_members()` from batch mode to per-user escrow (one MLS commit per invite). Added `list_pending_invites`, `accept_invite`, `decline_invite`, `handle_invite_declined` (auto-rotates keys to evict phantom MLS leaf).
- **Commands**: Added `/invites`, `/accept [id]`, `/decline <id>` commands with parser and tests.
- **TUI**: Added handlers for `Invites`, `Accept`, `Decline` commands. Added SSE handlers for `InviteReceived` and `InviteDeclined` events (with auto key rotation on decline).
- **GUI**: Added handlers for `Invites`, `Accept`, `Decline` commands. Added SSE handlers for both new event types.
- **Security**: Added upload_commit escrow bypass prevention. Added membership pre-check in accept_pending_invite. Proper UNIQUE constraint error mapping. All endpoints authenticate and authorize correctly.
- **Tests**: Added 34 new tests (7 DB, 21 API, 7 command parser). Total: ~496 tests across workspace.

### Files Modified

- `proto/conclave/v1/conclave.proto`
- `crates/conclave-server/src/db.rs`
- `crates/conclave-server/src/api.rs`
- `crates/conclave-server/src/main.rs`
- `crates/conclave-server/tests/api_tests.rs`
- `crates/conclave-client/src/api.rs`
- `crates/conclave-client/src/operations.rs`
- `crates/conclave-client/src/command.rs`
- `crates/conclave-cli/src/tui/commands.rs`
- `crates/conclave-cli/src/tui/events.rs`
- `crates/conclave-gui/src/app.rs`
- `crates/conclave-gui/src/subscription.rs`
- `docs/SPEC.md`
- `CLAUDE.md`

## 2026-02-22: Introduce Group Admin Role System

Replaced the single `creator_id` ownership model with a proper admin role system. The `creator_id` column was removed from the `groups` table. A `role TEXT NOT NULL DEFAULT 'member'` column was added to `group_members`. The group creator starts as admin. Admins can promote/demote other members. The last admin cannot be demoted (deadlock prevention).

### Changes

- **Database**: Removed `creator_id` from `groups` table. Added `role` column to `group_members`. Added `is_group_admin()`, `promote_member()`, `demote_member()`, `count_group_admins()`, `get_group_admins()` functions. Removed `get_group_creator()`.
- **API**: Added three new endpoints: `POST /groups/{id}/promote`, `POST /groups/{id}/demote`, `GET /groups/{id}/admins`. Updated `invite_to_group`, `remove_group_member`, and `update_group` to require admin instead of creator/member.
- **Protobuf**: Added `role` field to `GroupMember`, removed `creator_id` from `GroupInfo`. Added `PromoteMemberRequest/Response`, `DemoteMemberRequest/Response`, `ListAdminsResponse` messages.
- **Client**: Added `promote_member()`, `demote_member()`, `list_admins()` API methods. Added `role` field to `RoomMember` and `MemberInfo`. Added `/promote`, `/demote`, `/admins` commands.
- **TUI/GUI**: Added command handlers. Updated `/who` to show admin annotations. GUI member sidebar sorts admins first with indicator.
- **Tests**: Added 26 new tests (7 DB, 14 API, 5 command parser).

### Files Modified

- `proto/conclave/v1/conclave.proto`
- `crates/conclave-server/src/db.rs`
- `crates/conclave-server/src/api.rs`
- `crates/conclave-server/tests/api_tests.rs`
- `crates/conclave-client/src/api.rs`
- `crates/conclave-client/src/state.rs`
- `crates/conclave-client/src/operations.rs`
- `crates/conclave-client/src/command.rs`
- `crates/conclave-cli/src/tui/commands.rs`
- `crates/conclave-cli/src/tui/state.rs`
- `crates/conclave-gui/src/app.rs`
- `crates/conclave-gui/src/screen/dashboard.rs`
- `docs/SPEC.md`
- `CLAUDE.md`

## 2026-02-22: Fix Removed Creator Can Update Group

The `update_group` handler only checked that the requester was the group creator, not that they were still a member. A creator removed from a group could still rename it or change its alias. Added an `is_group_member` check alongside the existing creator check.

### Files Modified

- `crates/conclave-server/src/api.rs` — added membership check to `update_group`
- `crates/conclave-server/tests/api_tests.rs` — added `test_update_group_removed_creator_rejected`
- `docs/SPEC.md` — updated endpoint description

## 2026-02-22: Restrict Username and Group Name Characters

Tightened validation for usernames and group names to only allow alphanumeric characters and underscores (Telegram-style). Removed hyphens and dots from the allowed character set.

### Changes

- **Validation**: New pattern `^[a-zA-Z0-9][a-zA-Z0-9_]{0,63}$` for both usernames and group names. Extracted `validate_username()` function from inline code in the `register()` handler. Added `validate_group_name()` call to the `update_group()` handler (was previously unvalidated).
- **Tests**: Added 11 new rejection tests (username with dot/hyphen, group name with dot/hyphen/space/unicode/leading-underscore/empty/too-long, group name update with dot/hyphen). Replaced all hyphenated test names with underscore versions across 3 test files.

### Files Modified

- `crates/conclave-server/src/db.rs` — `validate_username()`, `validate_group_name()` updated
- `crates/conclave-server/src/api.rs` — use `validate_username()`, add validation to `update_group()`
- `crates/conclave-server/tests/api_tests.rs` — fix test names, add rejection tests
- `crates/conclave-server/tests/protocol_flow_tests.rs` — fix test names
- `docs/SPEC.md` — updated regex patterns

## 2026-02-22: GUI Draggable Sidebars

Added drag-to-resize for the left and right sidebars in the GUI dashboard. Transparent mouse-area overlays capture drag events globally during resize. Sidebar widths are clamped between 100px and 500px.

### Files Modified

- `crates/conclave-gui/src/screen/dashboard.rs` — drag state, handle rendering, overlay logic
- `crates/conclave-gui/src/app.rs` — DragStarted/DragUpdate/DragEnded message handlers
- `crates/conclave-gui/src/theme/container.rs` — drag_handle style

## 2026-02-22: Make group_name Mandatory

Made `group_name` a required field for group creation, matching the username model. Groups are identified by IDs internally; group_name is the unique human-readable identifier.

### Changes

- **Server DB**: `group_name TEXT UNIQUE` → `group_name TEXT UNIQUE NOT NULL`. Added `validate_group_name()` with same rules as username (`^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$`). Changed `create_group` signature from `Option<&str>` to `&str`. Changed `UserGroupRow.group_name` from `Option<String>` to `String`.
- **Server API**: Reject empty/invalid group names with 400 error in `create_group` handler.
- **Client lib**: Changed `api.rs`, `operations.rs` (`RoomInfo`), and `state.rs` (`Room`) — all `group_name` fields from `Option<String>` to `String`. Simplified `display_name()` methods.
- **TUI**: `/create <name>` now passes name as `group_name` (was incorrectly passing as `alias`).
- **GUI**: Updated `create_group` call signature. Simplified sidebar display to use non-optional `group_name`.
- **Tests**: Updated all group creation calls across api_tests.rs, protocol_flow_tests.rs, db.rs, and state.rs. Removed 4 tests for ID-fallback behavior that no longer applies.

### Files Modified

- `docs/SPEC.md` — schema NOT NULL, group name validation in security section
- `crates/conclave-server/src/db.rs` — schema, validation, signatures, UserGroupRow
- `crates/conclave-server/src/api.rs` — validation, response mapping
- `crates/conclave-server/tests/api_tests.rs` — unique group names in all tests
- `crates/conclave-server/tests/protocol_flow_tests.rs` — group creation helper
- `crates/conclave-client/src/api.rs` — create_group signature
- `crates/conclave-client/src/operations.rs` — RoomInfo, create_group, load_rooms
- `crates/conclave-client/src/state.rs` — Room struct, display_name
- `crates/conclave-cli/src/main.rs` — one-shot create-group command
- `crates/conclave-cli/src/tui/commands.rs` — /create passes group_name
- `crates/conclave-cli/src/tui/state.rs` — test helper make_room
- `crates/conclave-gui/src/app.rs` — create_group call, Room construction
- `crates/conclave-gui/src/screen/dashboard.rs` — sidebar display

## 2026-02-21: Logging Consistency Review

Reviewed all logging statements across the codebase for consistent style, tone, and punctuation. Fixed inconsistencies and added logging where it had debugging value.

### Fixes

1. **Structured fields**: Changed inline formatting (`"message: {e}"`) to structured fields (`error = %e, "message"`) in `error.rs` and `notification.rs`.
2. **Explicit field names**: Changed bare field shorthand (`user_id, count`) to explicit (`user_id = user_id, count = count`) in `api.rs` SSE lag warning.
3. **Path-qualified macros**: Removed `use tracing::warn;` import in `notification.rs`, switched to `tracing::warn!(...)`.
4. **GUI session cleanup**: Added `NotFound` check when removing session file during logout, matching CLI behavior (don't warn if the file is already gone).

### New logging

- `info` for user registration, login, and account reset (security-relevant events)
- `debug` for SSE client connections

### Documentation

- Added "Logging Conventions" section to `AGENTS.md` codifying the established patterns.

### Files Modified

- `crates/conclave-server/src/error.rs` — structured fields for database/internal errors
- `crates/conclave-server/src/api.rs` — explicit field names in SSE warning, added registration/login/reset/SSE logging
- `crates/conclave-client/src/notification.rs` — path-qualified macro, structured error field
- `crates/conclave-gui/src/app.rs` — skip warn on NotFound during session removal

## 2026-02-21: Fix TUI Not Re-rendering on Alias Update SSE Events

When another user changed their alias, the TUI called `load_rooms()` to update state but never re-rendered the screen because the "member_profile" SSE handler returned an empty message vec. The render loop only triggered on non-empty messages.

Fixed by calling `render_full()` when SSE event processing returns no display messages but state was updated.

### Files Modified

- `crates/conclave-cli/src/tui/mod.rs` — call `render_full` when SSE handler returns empty messages

## 2026-02-21: Auto-Login on Registration

All clients (CLI, TUI, GUI) now automatically log in and establish a full session after registration. Previously, users had to manually log in after registering.

### Changes

1. **GUI**: Register task returns `LoginInfo` instead of `RegisterInfo`. `handle_register_result` calls `handle_login_result` with a `skip_keygen` flag to avoid duplicate key package uploads (registration already generates and uploads them).

2. **CLI one-shot**: After registration and key package upload, the session is now saved (server URL, token, user ID, username) so the user is fully logged in.

3. **TUI**: After registration and key package upload, the TUI now updates shared state, saves session, initializes MLS, loads rooms, builds group mapping, and starts SSE — the same flow as `/login` but without re-generating key packages.

### Files Modified

- `crates/conclave-gui/src/app.rs` — Register returns LoginInfo, skip_keygen flag, removed RegisterInfo
- `crates/conclave-cli/src/main.rs` — Register saves session
- `crates/conclave-cli/src/tui/commands.rs` — Register does full login flow

## 2026-02-21: Split Server Config and Default Port by TLS Mode

Replaced the single `bind_address` config field with separate `listen_address` and `listen_port` fields. The port now defaults based on TLS mode: 8443 for HTTPS, 8080 for plain HTTP.

### Config format

```toml
listen_address = "0.0.0.0"    # default
listen_port = 8443             # optional; defaults to 8443 (TLS) or 8080 (HTTP)
```

### Changes

- `crates/conclave-server/src/config.rs` — Split into `listen_address: String` + `listen_port: Option<u16>`, added `socket_address()` method
- `crates/conclave-server/src/main.rs` — Use `config.socket_address()` instead of `config.bind_address`
- `docs/SPEC.md` — Updated config example and transport security docs

## 2026-02-21: Add MLS Epoch to Message Tooltip

Added MLS epoch tracking per message and rewrote the GUI tooltip format to show structured key-value metadata.

### Tooltip format

```
Timestamp: 2026-02-21 14:30:45
Sender ID: 42
Sender username: alice
Sender alias: Alice W.
Group ID: 3
Sequence: 157
Epoch: 5
```

Sender fields only shown for user messages. Alias line only shown when set. Removed the E2EE cipher suite display.

### Changes

1. **MLS epoch capture**: Added `MlsManager::group_epoch()` method. In `fetch_and_decrypt`, epoch is captured after each message decryption. In `send_message`, epoch is captured after encryption.

2. **Data pipeline**: Added `epoch: u64` to `ProcessedMessage` and `MessageSentResult`. Added `epoch: Option<u64>` to `DisplayMessage`. Added `epoch` column migration to local SQLite message store.

3. **Tooltip rewrite**: `format_tooltip()` now outputs labeled key-value lines instead of the previous compact format.

### Files Modified

- `crates/conclave-client/src/mls.rs` — Added `group_epoch()` method
- `crates/conclave-client/src/operations.rs` — Added `epoch` to `ProcessedMessage`, `MessageSentResult`, `fetch_and_decrypt`, `send_message`
- `crates/conclave-client/src/state.rs` — Added `epoch` to `DisplayMessage`
- `crates/conclave-client/src/store.rs` — DB migration, store/load epoch
- `crates/conclave-gui/src/widget/message_view.rs` — Rewrote tooltip format
- `crates/conclave-gui/src/app.rs` — Set epoch at conversion sites
- `crates/conclave-cli/src/tui/events.rs` — Set epoch at conversion site
- `crates/conclave-cli/src/tui/mod.rs` — Set epoch at startup message loading
- `crates/conclave-cli/src/tui/commands.rs` — Set epoch on sent messages

## 2026-02-21: Rich Message Tooltip with Extended Metadata

Extended the GUI message hover tooltip to show detailed metadata instead of just datetime + sender name.

### Tooltip now shows

- **Full datetime**: `2026-02-21 14:30:45`
- **Sender identity**: `Alice W. (@alice) | user#42` (alias, username, and user ID)
- **Message/group IDs**: `seq#157 | group#3` (sequence number and group ID)
- **Encryption info**: `E2EE: MLS (CURVE448_CHACHA)` (for user messages)

### Changes

1. **`DisplayMessage` extended**: Added `sequence_num: Option<u64>` field. Defaults to `None` in constructors; set from `ProcessedMessage.sequence_num` at conversion sites.

2. **`MessageStore` migration**: Added `sequence_num` column to local SQLite DB. Persisted and loaded alongside other message fields.

3. **Tooltip format**: `format_tooltip()` now builds a multi-line tooltip using sender identity from the room member list (alias vs username distinction), message sequence number, group ID, and MLS cipher suite.

4. **`message_list()` accepts `group_id: Option<i64>`**: Passed from `view_messages()` in the dashboard, which gets it from `active_room`.

### Files Modified

- `crates/conclave-client/src/state.rs` — Added `sequence_num` to `DisplayMessage`
- `crates/conclave-client/src/store.rs` — DB migration, store/load sequence_num
- `crates/conclave-gui/src/widget/message_view.rs` — Rich multi-line tooltip, accept group_id
- `crates/conclave-gui/src/screen/dashboard.rs` — Pass active_room as group_id to message_list
- `crates/conclave-gui/src/app.rs` — Set sequence_num when converting ProcessedMessage
- `crates/conclave-cli/src/tui/events.rs` — Set sequence_num when converting ProcessedMessage
- `crates/conclave-cli/src/tui/mod.rs` — Set sequence_num when loading messages at startup

## 2026-02-21: Broadcast Alias Changes to Other Clients via SSE

When a user changed their alias via `/nick`, other connected clients were never notified — only the user's own client refreshed. Added server-side SSE broadcast so alias changes propagate in real-time.

### Changes

1. **Server broadcast**: The `PATCH /api/v1/me` handler now broadcasts `GroupUpdateEvent` with `update_type: "member_profile"` to all co-members across all groups the user belongs to.

2. **TUI room refresh on GroupUpdate**: The TUI's `GroupUpdate` SSE handler now calls `commands::load_rooms()` to refresh member lists. For `"member_profile"` events, no system message is shown (alias changes are silent).

3. **GUI already handled**: The GUI already called `load_rooms_task()` on `GroupUpdate` events, so no GUI changes were needed.

### Files Modified

- `crates/conclave-server/src/api.rs` — Add SSE broadcast in `update_profile` handler
- `crates/conclave-cli/src/tui/events.rs` — Refresh rooms on `GroupUpdate`, suppress system message for `"member_profile"`
- `crates/conclave-server/tests/api_tests.rs` — Add `setup_with_state()` helper and 2 broadcast tests

## 2026-02-21: Store sender_id in Messages, Resolve Display Names at Render Time

Refactored message handling to store `sender_id` instead of baked-in sender name strings. Display names are now resolved dynamically from the room member list at render time. This means alias changes via `/nick` are retroactively reflected in all previous messages. Nick colors are now based on `sender_id` for stability across alias changes.

### Changes

1. **`DisplayMessage` and `ProcessedMessage`**: Added `sender_id: Option<i64>` (None for system messages). The `sender` field is kept as a fallback for when the sender can't be resolved from the member list.

2. **`MessageStore` schema**: Added `sender_id INTEGER NOT NULL DEFAULT 0` column to the `messages` table with a migration for existing databases. Messages are stored and loaded with sender_id.

3. **Render-time name resolution**: Added `resolve_sender_name()` helper in `state.rs` that looks up the sender's display name from the room member list by user_id. Both GUI and TUI use this at render time instead of the stored sender string.

4. **Stable nick colors**: `nick_color()` (GUI) and `username_color()` (TUI) now take `sender_id: i64` instead of a username string, ensuring colors remain consistent when aliases change.

5. **Removed `resolve_self_display_name()`** from TUI commands — no longer needed since display names are resolved at render time.

### Files Modified

- `crates/conclave-client/src/state.rs` — Added `sender_id` to `DisplayMessage`, `resolve_sender_name()` helper
- `crates/conclave-client/src/store.rs` — DB migration, store/load sender_id
- `crates/conclave-client/src/operations.rs` — Added `sender_id` to `ProcessedMessage`
- `crates/conclave-gui/src/app.rs` — Pass sender_id in message creation
- `crates/conclave-gui/src/widget/message_view.rs` — Accept members, resolve names at render time
- `crates/conclave-gui/src/screen/dashboard.rs` — Pass members to message_list
- `crates/conclave-gui/src/theme/mod.rs` — `nick_color(i64)` instead of `nick_color(&str)`
- `crates/conclave-cli/src/tui/commands.rs` — Pass sender_id, remove `resolve_self_display_name()`
- `crates/conclave-cli/src/tui/events.rs` — Pass sender_id from ProcessedMessage
- `crates/conclave-cli/src/tui/mod.rs` — Pass sender_id in startup message loading
- `crates/conclave-cli/src/tui/render.rs` — Accept members, resolve names, `username_color(i64)`
- `crates/conclave-cli/src/tui/state.rs` — Updated test calls
- `docs/SPEC.md` — Updated message_history.db description

## 2026-02-21: Fix Alias Display for Self-Sent Messages and Add Message Hover Tooltip

Fixed a bug where self-sent messages always showed the login username instead of the user's alias. Added a hover tooltip to messages in the GUI showing full date/time metadata.

### Changes

1. **Fix alias display for self-sent messages**: In the GUI, `handle_message_sent()` now uses `user_alias` (if set) instead of `username` for the sender display name. In the TUI, a `resolve_self_display_name()` helper looks up the current user's display name from the room member list (which is refreshed after `/nick`), falling back to username.

2. **Message hover tooltip**: Each message in the GUI now shows a tooltip on hover (300ms delay) with the full date and time (e.g., "2026-02-21 14:30:45") and sender name. Added a `tooltip` container style for consistent theming.

### Files Modified

- `crates/conclave-gui/src/app.rs` — Use alias for self-sent message sender display
- `crates/conclave-gui/src/widget/message_view.rs` — Wrap messages in tooltip showing full timestamp
- `crates/conclave-gui/src/theme/container.rs` — Add tooltip container style
- `crates/conclave-cli/src/tui/commands.rs` — Add `resolve_self_display_name()` helper, use for sent messages

## 2026-02-21: GUI Display Improvements and Alias Commands

Added `/nick` and `/topic` IRC-standard commands, improved display formatting across the GUI.

### Changes

1. **`/nick <alias>` command**: Sets the current user's display name via `PATCH /api/v1/me`. Available in GUI, TUI, and CLI one-shot mode. The user's alias is fetched from the server at login and session restore, then displayed in the sidebar user button and popover.

2. **`/topic <text>` command**: Sets the active room's display alias via `PATCH /api/v1/groups/{id}`. Available in GUI, TUI, and CLI one-shot mode. Requires the user to be the group creator.

3. **API client additions**: Added `patch()` HTTP helper, `update_profile()`, and `update_group()` methods to `ApiClient`.

4. **Members sidebar header**: The right sidebar now shows "N Members" as a header above the member list, matching the "Rooms" header style on the left sidebar.

5. **Room list format**: Changed from `# displayname` to `alias (#groupname)` when an alias is set, or `#groupname` when no alias is set. Uses the room's unique name (group_name or numeric ID) as the `#identifier`.

6. **Member list format**: Changed from just the display name to `alias (@username)` when an alias is set, or `@username` when no alias is set.

7. **User button format**: Changed from `@username` to `alias (@username)` when the user has an alias set.

8. **User popover restructure**: Split the single `username@server_url` line into up to 4 lines: alias (if set), @username, user#ID, and server URL. The popover now resizes dynamically to fit content.

### Files Modified

- `crates/conclave-client/src/api.rs` — Added `patch()`, `update_profile()`, `update_group()`
- `crates/conclave-client/src/command.rs` — Added `Nick`, `Topic` command variants with parsing and tests
- `crates/conclave-gui/src/app.rs` — Added `user_alias` field, fetch at login/restore, `/nick` and `/topic` handlers, updated `view()` and `show_help()`
- `crates/conclave-gui/src/screen/dashboard.rs` — Members sidebar header, room/member/user display format changes, popover restructure
- `crates/conclave-cli/src/tui/commands.rs` — `/nick` and `/topic` command handlers, updated help text
- `crates/conclave-cli/src/main.rs` — `Nick` and `Topic` CLI subcommands

## 2026-02-21: Server-Side Group Mapping and Registration Key Packages

Three interrelated fixes making the system more robust: server-side group mapping storage, registration key package upload, and `/reset` reliability.

### Changes

1. **Server-side MLS group ID storage**: The `groups` table now has an `mls_group_id TEXT` column. The server returns `mls_group_id` in the `ListGroupsResponse` (`GroupInfo` message) and stores it from `UploadCommitRequest` (on group creation) and `ExternalJoinRequest` (on rejoin). Clients build their in-memory group mapping from the server response on login/reconnect instead of relying on the local `group_mapping.toml` file. A local file fallback exists for migration (groups created before the server stored `mls_group_id`).

2. **Registration uploads key packages**: All three registration paths (CLI one-shot, TUI, GUI) now auto-login after registration, create an MLS identity, and upload initial key packages (1 last-resort + 5 regular). Previously, newly registered users couldn't be invited to groups until they logged out and back in.

3. **`/reset` fetches groups from server**: `reset_account()` now calls `load_rooms()` to discover groups from the server instead of relying on the local `group_mapping` parameter. This fixes `/reset` showing "rejoined 0/0 groups" when the local data directory was lost (the scenario that makes reset necessary). The function also passes `mls_group_id` in `external_join()` calls so the server stores the new MLS group ID after rejoin.

4. **TUI/GUI stop writing `group_mapping.toml`**: The TUI and GUI no longer call `save_group_mapping()`. The mapping is ephemeral in memory, rebuilt from the server on each login/reconnect. One-shot CLI commands still read/write the file for backward compatibility.

### Files Modified

- `proto/conclave/v1/conclave.proto` — Added `mls_group_id` to `GroupInfo`, `UploadCommitRequest`, `ExternalJoinRequest`
- `crates/conclave-server/src/db.rs` — Added `mls_group_id TEXT` column, migration, `set_mls_group_id()`, updated `list_user_groups()` return type
- `crates/conclave-server/src/api.rs` — `list_groups` returns `mls_group_id`; `upload_commit`/`external_join` store it
- `crates/conclave-client/src/api.rs` — Added `mls_group_id` param to `upload_commit()` and `external_join()`
- `crates/conclave-client/src/operations.rs` — Added `mls_group_id` to `RoomInfo`; rewrote `reset_account()` to fetch groups from server
- `crates/conclave-client/src/config.rs` — Added `build_group_mapping()` helper
- `crates/conclave-cli/src/main.rs` — Registration auto-login + key packages; kept file I/O for one-shot commands
- `crates/conclave-cli/src/tui/commands.rs` — Registration auto-login + key packages; login builds mapping from server; removed `save_group_mapping` calls
- `crates/conclave-cli/src/tui/mod.rs` — Build mapping from server rooms instead of file
- `crates/conclave-cli/src/tui/events.rs` — Removed `save_group_mapping` calls
- `crates/conclave-gui/src/app.rs` — Registration auto-login + key packages; build mapping from server; removed all `save_group_mapping` calls; updated reset flow
- `crates/conclave-server/tests/api_tests.rs` — Added `mls_group_id` field to 21 proto struct initializers
- `crates/conclave-server/tests/protocol_flow_tests.rs` — Added `mls_group_id` field to 2 proto struct initializers

### Verification

- `cargo build --release` — clean
- `cargo test --workspace` — all 418 tests pass
- `cargo clippy --workspace` — no new warnings
- `cargo fmt --all -- --check` — clean

## 2026-02-21: Add Identity Reset Notifications

Added Signal/Matrix-style identity change warnings. When a user resets their encryption identity (via `/reset` or after data loss), all shared rooms now display a clear warning to other members.

### Changes

1. **New `IdentityResetEvent` SSE event**: Added to protobuf schema (`ServerEvent` oneof field 5). Carries `group_id` and `username` of the user who reset.

2. **Server `external_join` handler**: Now sends `IdentityResetEvent` instead of generic `GroupUpdateEvent` when a user rejoins via external commit. Looks up the resetting user's username for the notification.

3. **Client handling (CLI + GUI)**: Both frontends process the new event by showing a warning message ("{username} has reset their encryption identity. New messages are secured with their new keys.") and processing the underlying external commit to advance MLS epoch state.

4. **Login-time stale group detection**: When a user logs in and the server returns groups that have no local MLS mapping, both CLI and GUI now show a warning suggesting `/reset` to rejoin with a new identity.

### Files Modified

- `proto/conclave/v1/conclave.proto` -- `IdentityResetEvent` message + `ServerEvent` variant
- `crates/conclave-server/src/api.rs` -- `external_join` sends `IdentityResetEvent`
- `crates/conclave-client/src/operations.rs` -- `SseEvent::IdentityReset` variant + decode + test
- `crates/conclave-cli/src/tui/events.rs` -- Handle `IdentityReset` SSE
- `crates/conclave-cli/src/tui/commands.rs` -- Stale group detection on login
- `crates/conclave-gui/src/subscription.rs` -- `SseUpdate::IdentityReset` variant + decode
- `crates/conclave-gui/src/app.rs` -- Handle `IdentityReset` SSE + stale group detection

## 2026-02-21: Fix Group Mapping and Adapt ID Types to Integer Design

Fixed multiple bugs from the incomplete UUID-to-integer ID migration that prevented group joining and message processing.

### Changes

1. **`group_mapping` type**: Changed from `HashMap<String, String>` to `HashMap<i64, String>` across all crates. Eliminated ~25 `.to_string()` calls at mapping lookup/insertion sites. The TOML crate round-trips `HashMap<i64, String>` correctly; existing `group_mapping.toml` files remain compatible.

2. **`user_id` type**: Changed proto fields from `uint64` to `int64` for user IDs (RegisterResponse, LoginResponse, GroupInfo.creator_id, GroupMember.user_id, StoredMessage.sender_id, NewMessageEvent.sender_id, UserInfoResponse.user_id). Updated `SessionState.user_id`, `RoomMember.user_id`, and `MemberInfo.user_id` from `u64` to `i64`. Eliminated ~30 `as i64`/`as u64` casts across all crates.

3. **Stale room pruning**: `load_rooms` in both CLI and GUI now prunes rooms that the server no longer returns (e.g., user removed while offline). Stale entries are removed from both `rooms` and `group_mapping`, and `active_room` is cleared if it was stale.

4. **Test updates**: Updated config.rs tests to use integer keys, server test helpers to return `i64`, and removed `as i64` casts from 20 sites in protocol_flow_tests.

### Files Modified

- `proto/conclave/v1/conclave.proto` — `uint64` → `int64` for user_id fields
- `crates/conclave-client/src/config.rs` — `HashMap<i64, String>`, `user_id: Option<i64>`, tests
- `crates/conclave-client/src/state.rs` — `RoomMember.user_id: i64`
- `crates/conclave-client/src/operations.rs` — `MemberInfo.user_id: i64`, `ResetResult.new_group_mapping: HashMap<i64, String>`
- `crates/conclave-cli/src/main.rs` — Removed casts and `.to_string()` calls
- `crates/conclave-cli/src/tui/{state,commands,events,mod}.rs` — `HashMap<i64, String>`, `user_id: Option<i64>`, stale room pruning
- `crates/conclave-gui/src/app.rs` — Same type changes, stale room pruning
- `crates/conclave-server/src/api.rs` — Removed `as u64` casts
- `crates/conclave-server/tests/{api_tests,protocol_flow_tests}.rs` — `u64` → `i64` in helpers

## 2026-02-21: Add Test Coverage for ID/Naming Redesign

Added 59 new tests across 6 files to cover pure functions and new API endpoints introduced by the ID/naming redesign. Total workspace tests: 417.

- **`crates/conclave-client/src/state.rs`** — 10 tests: `RoomMember::display_name()`, `Room::display_name()`, `DisplayMessage` factories
- **`crates/conclave-client/src/operations.rs`** — 19 tests: `RoomInfo::display_name()`, `MemberInfo::display_name()`, `MemberInfo::to_room_member()`, `decode_sse_event()` (all event types + invalid/empty), `resolve_user_display_name()`
- **`crates/conclave-client/src/api.rs`** — 8 tests: `normalize_server_url()` (scheme handling, trailing slashes, edge cases)
- **`crates/conclave-server/tests/api_tests.rs`** — 10 tests: `PATCH /api/v1/me` (update/clear/invalid alias, unauth), `PATCH /api/v1/groups/{id}` (alias, group_name, non-creator rejected, not found, duplicate name, invalid alias)
- **`crates/conclave-cli/src/tui/state.rs`** — 4 tests: `resolve_room()` edge cases (empty input, numeric group_name vs ID, both alias and group_name searchable, multiple prefix matches)
- **`crates/conclave-server/src/db.rs`** — 8 tests: alias validation edge cases (control chars, tab, newline, unicode, clear alias to None, validation on update)

## 2026-02-21: Redesign User/Group ID and Naming System

### What Changed

Complete redesign of the identifier system for users and groups. Users and groups are now primarily identified by auto-increment integer IDs. Users have a required unique username (for auth and discovery) and an optional non-unique alias (display name). Groups have an optional unique group_name (for discovery) and an optional non-unique alias (display name). Similar to Telegram's model.

Display name resolution: Users: alias > username. Groups: alias > group_name > id.to_string().

This is a breaking change affecting all 5 workspace crates, the protobuf schema, database schema, MLS credentials, and both client frontends.

#### Protobuf Schema (`proto/conclave/v1/conclave.proto`)

- All `string group_id` fields changed to `int64 group_id` across all messages
- `RegisterRequest`: added `string alias = 3`
- `LoginResponse`: added `string username = 3`
- `UserInfoResponse`: added `string alias = 3`
- `CreateGroupRequest`: field 1 renamed to `alias`, added `string group_name = 3`
- `GroupInfo`: renamed `name` to `alias`, added `string group_name = 6`
- `GroupMember`: added `string alias = 3`
- `PendingWelcome`/`WelcomeEvent`: renamed `group_name` to `group_alias`
- `StoredMessage`: added `string sender_alias = 6`
- Added `UpdateProfileRequest/Response` and `UpdateGroupRequest/Response` messages

#### Server Database (`crates/conclave-server/src/db.rs`)

- `users` table: added `alias TEXT` column
- `groups` table: `id` changed from `TEXT PRIMARY KEY` to `INTEGER PRIMARY KEY AUTOINCREMENT`, `name` renamed to `alias` (nullable), added `group_name TEXT UNIQUE`
- All `group_id` columns changed from `TEXT` to `INTEGER` across `group_members`, `messages`, `pending_welcomes`, `group_infos` tables
- `pending_welcomes.group_name` renamed to `group_alias` (nullable)
- All method signatures changed from `group_id: &str` to `group_id: i64`
- New methods: `validate_alias()`, `update_user_alias()`, `update_group_alias()`, `update_group_name()`, `get_group_alias()`, `get_group_creator()`
- `get_group_members()` now returns alias alongside user_id and username
- Removed `uuid` dependency

#### Server API (`crates/conclave-server/src/api.rs`)

- All `Path<String>` changed to `Path<i64>` for group endpoints
- New routes: `PATCH /api/v1/me` (update user alias), `PATCH /api/v1/groups/{id}` (update group alias/name, creator only)
- `register`: accepts optional alias
- `login`: includes username in response
- `list_groups`: populates alias, group_name, and member aliases in GroupInfo
- Added `Validation(String)` variant to server error enum

#### Client MLS (`crates/conclave-client/src/mls.rs`)

- `MlsManager::new()` takes `user_id: i64` instead of `username: &str`
- `BasicCredential` now stores `user_id.to_be_bytes()` (8 bytes, big-endian i64) instead of username bytes
- `extract_username_from_identity()` replaced by `extract_user_id_from_identity() -> Option<i64>`
- `CommitInfo.members_added`: `Vec<String>` changed to `Vec<Option<i64>>`
- `GroupDetails.members`: `Vec<(u32, String)>` changed to `Vec<(u32, Option<i64>)>`
- `find_member_index()` takes `user_id: i64` instead of `username: &str`

#### Client API (`crates/conclave-client/src/api.rs`)

- All group methods: `group_id: &str` changed to `group_id: i64`
- `register()`: added `alias: Option<&str>` parameter
- `create_group()`: changed from `name: &str` to `alias: Option<&str>, group_name: Option<&str>`

#### Client State (`crates/conclave-client/src/state.rs`)

- Added `RoomMember` struct with `user_id`, `username`, `alias`, and `display_name()` method
- `Room.server_group_id`: `String` changed to `i64`
- `Room.name`: replaced by `alias: Option<String>` and `group_name: Option<String>`
- `Room.members`: `Vec<String>` changed to `Vec<RoomMember>`
- Added `Room::display_name()` for alias > group_name > id fallback

#### Client Operations (`crates/conclave-client/src/operations.rs`)

- New types: `RoomInfo`, `MemberInfo` with display name resolution
- All result types use `i64` group IDs
- All functions take `user_id: i64` instead of `username: &str`
- `fetch_and_decrypt`: added `members: &[RoomMember]` param for display name resolution
- `kick_member`: takes `target_user_id: i64`

#### Client Store (`crates/conclave-client/src/store.rs`)

- Schema: `group_id` changed from `TEXT` to `INTEGER` in both tables
- All methods: `group_id: &str` changed to `group_id: i64`

#### CLI TUI (`crates/conclave-cli/src/tui/`)

- `rooms: HashMap<String, Room>` changed to `HashMap<i64, Room>`
- `active_room: Option<String>` changed to `Option<i64>`
- Room resolution: searches alias, group_name, prefix match, then i64 parse
- Display uses `room.display_name()` and `member.display_name()`
- `/kick` resolves target_user_id from room member list

#### GUI (`crates/conclave-gui/src/`)

- Same HashMap key changes as CLI (String -> i64)
- `Message::RoomSelected(String)` changed to `RoomSelected(i64)`
- Sidebar and title bar use `display_name()` methods

#### Tests

- All 358 tests pass across the workspace
- Server DB tests updated for integer group IDs, alias validation, group_name uniqueness
- Server API tests updated for new proto fields and integer endpoints
- Protocol flow tests updated for i64 user_id in MLS credentials
- Client MLS tests updated for user_id-based credentials

#### Breaking Changes

1. Wire format: all group_id fields changed from string to int64 in protobuf
2. MLS credentials: BasicCredential now contains user_id bytes instead of username (existing MLS groups incompatible, users must `/reset`)
3. Client-side storage: group_mapping.toml keys changed from UUIDs to integers; message_history.db schema changed
4. Server database: schema changes require a fresh database (acceptable for pre-1.0)

#### Verification

- `cargo build --release` -- clean
- `cargo test --workspace` -- all 358 tests pass
- `cargo clippy --workspace` -- no new warnings
- `cargo fmt --all -- --check` -- clean

## 2026-02-20: Consolidate Business Logic into Shared Operations Module

### What Changed

Extracted all duplicated business logic from the CLI (`conclave-cli`) and GUI (`conclave-gui`) into a new `operations` module in the shared library (`conclave-client`). Both clients are now thin UI shells that delegate all protocol orchestration to the shared library.

#### New Module: `crates/conclave-client/src/operations.rs`

12 public functions and 8 result types covering all MLS-over-HTTP orchestration:

**Result types**: `RoomInfo`, `ProcessedMessage`, `FetchedMessages`, `GroupCreatedResult`, `WelcomeJoinResult`, `MessageSentResult`, `ResetResult`, `SseEvent`.

**Functions**:
- `decode_sse_event` — Hex+protobuf SSE event decoding
- `load_rooms` — Fetch and normalize group list from server
- `fetch_and_decrypt` — Fetch messages after a sequence number and decrypt via MLS
- `send_message` — MLS encrypt + API send
- `create_group` — API create + MLS group creation + commit/welcome upload
- `invite_members` — API invite + MLS invite + commit/welcome upload
- `kick_member` — MLS find member index + remove + API notify
- `rotate_keys` — MLS epoch advancement + commit upload
- `leave_group` — MLS self-remove commit + API leave + delete local state
- `delete_mls_group_state` — Delete local MLS group state for a single group
- `accept_welcomes` — Process pending welcomes + MLS join + key package replenishment
- `reset_account` — Full account reset: collect indices, wipe state, regen identity, rejoin all groups via external commit

All functions use `tokio::task::spawn_blocking` for MLS operations (MlsManager is not `Send`) and propagate errors via `conclave_client::error::Result`.

#### CLI Changes

**`crates/conclave-cli/src/main.rs`**:
- Extracted `api_from_session`, `require_username`, `resolve_mls_group_id` helpers
- Refactored 6 one-shot commands (CreateGroup, Invite, Groups, Join, Send, Messages) to use `operations::*`

**`crates/conclave-cli/src/tui/commands.rs`**:
- Refactored 10 TUI commands (Create, Join, Invite, Kick, Leave, Rotate, Reset, Msg, Message, load_rooms) to use `operations::*`

**`crates/conclave-cli/src/tui/events.rs`**:
- Rewrote `handle_sse_message` to use `operations::decode_sse_event`
- Rewrote `handle_new_message`, `handle_welcome`, `handle_member_removed` to use `operations::fetch_and_decrypt`, `operations::accept_welcomes`, `operations::delete_mls_group_state`

**`crates/conclave-cli/src/tui/mod.rs`**:
- Rewrote `accept_pending_welcomes` and `fetch_missed_messages` to use `operations::*`

#### GUI Changes

**`crates/conclave-gui/src/app.rs`**:
- Removed 7 local type definitions (`RoomInfo`, `MessageSentInfo`, `FetchedMessages`, `DecryptedMsg`, `WelcomeResult`, `GroupCreatedInfo`, `ResetCompleteInfo`) — replaced by `operations::*` types
- Updated all 6 Message enum variants and their handler functions to use `operations::*` types
- Refactored 6 business logic methods (`invite_members`, `kick_member`, `leave_group`, `rotate_keys`, `reset_account`, `accept_welcomes`) to call `operations::*`
- Replaced `MemberRemoved` SSE handler's inline MLS logic with `operations::delete_mls_group_state`
- Added `load_rooms_task()` and `fetch_messages_task()` helper methods to eliminate boilerplate
- Replaced all 5 `load_rooms_async` call sites and 2 `fetch_and_decrypt` call sites
- Removed both free functions (`load_rooms_async`, `fetch_and_decrypt`) — ~120 lines of duplicated logic

**`crates/conclave-gui/src/subscription.rs`**:
- Replaced local `decode_sse_event` with `operations::decode_sse_event`
- Removed unused `prost::Message` import

#### Verification

- `cargo build --workspace` — clean
- `cargo test --workspace` — all 330 tests pass
- `cargo clippy --workspace` — no new warnings
- `cargo fmt --all -- --check` — clean

## 2026-02-20: Fix Missing Messages for Groups Joined While Offline

### What Changed

When the GUI user was offline and another user created a group and invited them, upon coming back online the GUI would join the group but not display messages sent before coming online.

#### Root Cause (two issues)

**Issue 1 — Sequence number over-advance**: During welcome processing, `accept_welcomes()` fetched the maximum sequence number for each newly joined group and stored it as `last_seen_seq` via a `welcome_seqs` map. This was applied to rooms in `handle_rooms_loaded()`, causing `fetch_all_missed_messages()` to request messages after the max sequence — returning nothing. All pre-existing messages (including ones sent while the user was offline) were skipped. The CLI had the same `max_seq` calculation in `accept_pending_welcomes()` but it was accidentally overridden by the store restoration loop (which reset `last_seen_seq` to the persisted value of 0 for new groups), so the CLI was unaffected.

**Issue 2 — Race condition in deferred fetch**: `handle_welcomes_processed()` called `fetch_all_missed_messages()` concurrently with a `rooms_task` reload. Since `fetch_all_missed_messages()` iterates over `self.rooms`, which didn't yet contain the newly joined groups (only populated when the reload completes), the fetch missed them entirely.

#### Fix

**`crates/conclave-gui/src/app.rs`**:
- Removed the `welcome_seqs` field and all related `last_seen_seq` over-advancement logic. For newly joined groups, `last_seen_seq` now defaults to 0 (from the message store, which has no entry), so `fetch_all_missed_messages()` fetches from the beginning. Commits are handled gracefully by the `DecryptedMessage` variants.
- Removed the `last_seen_seq` field from `WelcomeResult` and the `get_messages()` call in `accept_welcomes()` that computed `max_seq`.
- Added `fetch_messages_on_rooms_load` flag. `handle_welcomes_processed()` now sets this flag instead of calling `fetch_all_missed_messages()` concurrently. `handle_rooms_loaded()` checks the flag after rooms are populated and triggers the fetch at that point.

**`crates/conclave-cli/src/tui/mod.rs`**:
- Removed the dead `max_seq` calculation in `accept_pending_welcomes()` that was always overridden by the store restoration loop.

**`crates/conclave-client/src/mls.rs`**:
- In `decrypt_message()`, replaced fragile string matching (`err_str.contains(...)`) with structured `MlsError` enum matching (`MlsError::CantProcessMessageFromSelf`, `MlsError::InvalidEpoch`). Both variants return `DecryptedMessage::None` (silently skip). `InvalidEpoch` handles messages from epochs before the client joined (e.g., the group-creation commit when joining via welcome) — these lack key material and cannot be decrypted. The only other string-based error matching in the codebase (rusqlite "duplicate column" in `db.rs` and `store.rs`) was investigated but rusqlite has no structured error code for this case.

## 2026-02-19: Fix GUI SSE "Always Disconnected" Bug

### What Changed

The GUI always showed "Disconnected" status even though API calls (creating rooms, sending messages) worked fine. The CLI was unaffected.

#### Root Cause

`self.server_url` in the GUI stored the raw URL from the login form (e.g., `host:port`) without scheme normalization. The SSE subscription used this raw URL directly to construct the events endpoint (`host:port/api/v1/events`), which is invalid without `https://`. Meanwhile, `ApiClient::new()` internally normalized URLs by prepending `https://` when no scheme was present, so all API calls worked fine. The CLI was unaffected because it uses `api.connect_sse()` which uses the already-normalized `ApiClient.base_url`.

#### Fix

- **`crates/conclave-client/src/api.rs`**: Extracted URL normalization logic from `ApiClient::new()` into a public `normalize_server_url()` function, then refactored `ApiClient::new()` to use it.
- **`crates/conclave-gui/src/app.rs`**: Imported `normalize_server_url` and applied it when setting `self.server_url` in both the session restore path (`Conclave::new()`) and the login result handler (`handle_login_result()`). This ensures the SSE subscription always receives a properly normalized URL with scheme.

## 2026-02-19: GUI `/reset` Command Implementation

### What Changed

Ported the `/reset` command from the CLI TUI to the GUI client. The GUI previously displayed a stub message ("Reset not yet supported in GUI. Use CLI.") — now it performs the full RFC 9420-compliant account reset flow.

#### `crates/conclave-gui/src/app.rs`

- Added `ResetCompleteInfo` struct and `Message::ResetComplete` variant for the async result.
- Replaced the `Command::Reset` stub with `self.reset_account()`.
- Implemented `reset_account()` method: single `Task::perform()` async block that collects groups and old leaf indices, calls `api.reset_account()`, wipes local MLS state, regenerates identity, uploads new key packages (1 last-resort + 5 regular), and performs external commit rejoin for each group — matching the CLI flow exactly.
- Implemented `handle_reset_complete()` method: updates `self.group_mapping`, saves to disk, reinitializes `self.mls` with the new identity, clears stale `fetching_groups` state, displays status messages, and triggers a room reload.
- Added `/reset` to the `/help` output.

## 2026-02-18: Comprehensive Test Suite Expansion

### What Changed

Expanded the test suite from ~208 tests to 330 tests (+122 new tests) covering MLS protocol compliance, server API edge cases, database/auth internals, client storage, and end-to-end protocol flows with real MLS cryptography.

### Bug Fix: MLS Key Package Wire Format (RFC 9420 Section 6)

- **`conclave-server/src/api.rs`**: `validate_key_package_wire_format()` checked for wire format `3` (`mls_welcome`), but the correct value per RFC 9420 Section 6 is `5` (`mls_key_package`). Fixed to validate against `5`. Updated `docs/SPEC.md` accordingly.

### New Tests by Category

#### MLS Protocol Compliance (11 tests in `conclave-client/src/mls.rs`)
- Epoch retention boundary (16 epochs), five-member group operations, invite after multiple key rotations, removed member cannot rejoin via old welcome, external rejoin with self-removal, multi-group isolation, rapid sequential messages, binary payload roundtrip, leave group self-removal detection, group info epoch matching, concurrent key rotations from different members.

#### Server API Edge Cases (16 tests in `conclave-server/tests/api_tests.rs`)
- Username boundary validation (64-char max, starting with underscore/dot/hyphen, valid special chars, empty password), key package edge cases (exactly 16 KiB, batch oversized entry), external join without group info, message pagination cap at 500, group name exactly 128 chars, auth header format (missing Bearer prefix, empty bearer), upload commit with multiple welcomes, leave group stores group info, external join commit stored as message.

#### Server Database & Auth (14 tests in `conclave-server/src/db.rs` and `auth.rs`)
- `process_commit` with multiple welcomes, empty commit message, empty group info, nonexistent user. Messages isolated between groups, `group_exists`, multiple pending welcomes for same user, delete welcome wrong user, `count_key_packages`, session token hashed. Auth: dummy hash validity, token hex format, empty password hashing/verification.

#### Client Store & Config (13 tests in `conclave-client/src/store.rs` and `config.rs`)
- Room state creation via set_last_seen/read_seq, reopen preserves room state independently, empty/unicode/large content messages, system vs user message counts, sequence numbers isolated between groups. Config: group mapping empty values, many entries, malformed session file, key package structure verification.

#### End-to-End Protocol Flow (9 tests in `conclave-server/tests/protocol_flow_tests.rs`)
- Full group creation and bidirectional messaging with real MLS encryption/decryption through the server API. Three-party group messaging. Post-creation invite flow (solo group → invite → welcome → messaging). Member removal flow with commit processing by remaining members. Key rotation continuity (epoch advance preserves messaging). External rejoin after removal. Real key package wire format validation. Key package roundtrip through server (upload → retrieve → parse by another client). Message ordering and sequence number verification across 10 sequential messages.

### Test Counts

| Crate | Before | After |
|-------|--------|-------|
| conclave-cli | 33 | 33 |
| conclave-client | 116 | 140 |
| conclave-server (unit) | 40 | 54 |
| conclave-server (api_tests) | 78 | 94 |
| conclave-server (protocol_flow_tests) | 0 | 9 |
| **Total** | **~208** | **330** |

---

## 2026-02-18: Comprehensive Codebase Audit & Remediation

### What Changed

Full codebase audit covering RFC 9420 compliance, security vulnerabilities, Rust coding guideline adherence, and code quality. 22 issues identified (6 CRITICAL, 5 HIGH, 4 MEDIUM, 7 LOW) and remediated across 5 phases. All 190 tests pass after fixes.

### Phase 1: Critical Security & RFC 9420 Fixes

#### C1. `external_join` authorization check (`conclave-server/src/api.rs`)
- `external_join()` previously allowed any authenticated user to join any group. Added checks: group must exist (`group_exists()`), and a stored GroupInfo must be present (only set by authorized members via `upload_commit` or `remove_group_member`). Without GroupInfo, the endpoint returns 400.
- Added `group_exists()` method to `conclave-server/src/db.rs`.

#### H1. Key package exhaustion DoS (`conclave-server/src/api.rs`)
- Any user could drain another user's key packages by repeatedly calling `GET /api/v1/key-packages/{user_id}`. Added per-user rate limiting via an in-memory token bucket (`KeyPackageRateLimiter`) that allows 10 requests per minute per target user.

#### H5. Username character validation (`conclave-server/src/api.rs`)
- Registration only checked non-empty and max 64 chars. Added regex validation: `^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$`. Rejects control characters, Unicode homoglyphs, whitespace-only strings, and names starting with punctuation.

#### C2. `leave_group` MLS commit (`conclave-server/src/api.rs`, `conclave-client/src/mls.rs`, TUI/GUI)
- Leaving a group previously only removed the user from the server DB without producing an MLS commit. Remaining members' MLS state still included the departed member (RFC 9420 Section 12.3 violation).
- `MlsManager::leave_group()` now produces a self-remove proposal+commit and returns `(commit_message, group_info)`.
- `LeaveGroupRequest` protobuf updated with `commit_message` and `group_info` fields.
- Server `leave_group` endpoint stores the commit as a message and updates group_info.
- TUI/GUI `MemberRemovedEvent` handlers fetch and process the leave commit to advance MLS epoch.

#### C3. `delete_group_state` actually deletes (`conclave-client/src/mls.rs`)
- `delete_group_state()` was a no-op (loaded group state and wrote it back unchanged). Now calls `SqLiteDataStorageEngine::delete_group()` to properly remove MLS cryptographic material.

#### C4. Key package replenishment (`conclave-cli/src/tui/commands.rs`, `conclave-gui/src/app.rs`)
- After accepting welcomes, only 1 replacement key package was uploaded regardless of how many were consumed. Now uploads N replacements (one per welcome consumed) to maintain the 5 regular + 1 last-resort pool.

#### C5. Server-side key package validation (`conclave-server/src/api.rs`)
- `upload_key_package` accepted any non-empty blob under 16 KiB. Added `validate_key_package_wire_format()` that checks the MLS 1.0 version header (0x0001) and mls_key_package wire format (0x0003) per RFC 9420 Section 6.

#### C6. Welcome-to-username mapping (`conclave-client/src/mls.rs`)
- `create_group()` and `invite_to_group()` assumed `commit_output.welcome_messages[i]` corresponded to `username_order[i]`, relying on mls-rs producing welcomes in add-order (not guaranteed).
- Now uses `key_package_reference()` and `welcome_key_package_references()` to match each welcome to its recipient by KeyPackage reference (RFC 9420 Section 12.4.3).

### Phase 2: High-Priority Security Fixes

#### H2. Atomic `upload_commit` (`conclave-server/src/api.rs`, `conclave-server/src/db.rs`)
- Multiple DB operations in `upload_commit` (add members, store welcomes, store group info, store message) were sequential without a transaction. Added `Database::process_commit()` that wraps all operations in a SQLite savepoint. SSE notifications are sent only after the transaction commits.

#### H3. Login timing equalization (`conclave-server/src/api.rs`, `conclave-server/src/auth.rs`)
- Non-existent user path called `hash_password()` (different computational profile from `verify_password()`), enabling username enumeration via timing. Now both paths call `verify_password()` — the non-existent path uses a precomputed `DUMMY_HASH` (`LazyLock<String>`) generated at startup.

#### H4. GUI logout token revocation (`conclave-gui/src/app.rs`)
- `perform_logout()` cleared local state but never called `api.logout()` to revoke the server-side session. Now fires an async `api.logout()` task before clearing local state.

#### L6. New members excluded from commit notifications (`conclave-server/src/api.rs`)
- `upload_commit` sent `GroupUpdateEvent` to all members including newly added ones who should process their Welcome first. Now tracks `new_member_ids` during welcome processing and excludes them from the commit notification.

### Phase 3: Protocol Robustness

#### L4. SSE lagged clients (`conclave-server/src/api.rs`)
- The SSE stream's `_ => None` catch-all silently dropped `BroadcastStream::Lagged` errors. Now matches `Lagged` explicitly and sends a special "lagged" SSE event so clients know to re-sync.

#### L5. Sequence number uniqueness (`conclave-server/src/db.rs`)
- `SELECT MAX + INSERT` for sequence numbers was not atomic. Added `UNIQUE(group_id, sequence_num)` constraint to the messages table schema.

#### L7. `fetching_groups` not cleared on error (`conclave-gui/src/app.rs`)
- On fetch error, the `group_id` was not removed from `fetching_groups`, permanently blocking future fetches. Changed `MessagesFetched` error type from `String` to `(String, String)` carrying the group_id so it's always cleared.

### Phase 4: Error Handling & Async Correctness

#### M1. `unwrap()`/`expect()` removal (10 instances across 4 crates)
- `proto_response()`: returns HTTP 500 fallback on encode failure.
- `unix_now()`: uses `unwrap_or_default()`.
- 6 SSE event encoding `unwrap()`s: replaced with `if let Err` + tracing.
- `error.rs` encode `unwrap()`: replaced with `if let Err` + tracing.
- Client `Client::builder().build().expect()`: replaced with `unwrap_or_default()`.
- Client `body.encode().unwrap()`: replaced with `?` propagation.
- GUI icon loading `expect()`: replaced with `.ok()`.
- GUI subscription `expect()`: replaced with `unwrap_or_default()`.

#### M2. `let _ =` on fallible operations (~25 instances)
- `wipe_local_state` file deletions: now logs non-`NotFound` errors at warn level.
- Server/client DB migrations: check specifically for "duplicate column" error, propagate others.
- Store operations (`push_message`, `set_last_seen_seq`, `set_last_read_seq`): trace-level logging.
- `accept_welcome` calls: warn-level logging.
- TUI logout/session file removal: warn-level logging with `NotFound` check.

#### L3. TUI async correctness (`conclave-cli/src/tui/commands.rs`)
- `encrypt_message` MLS calls blocked the tokio runtime thread. Wrapped in `tokio::task::spawn_blocking` for both `Command::Msg` and `Command::Message` paths.

### Phase 5: Code Quality Cleanup

#### L1/L2. Deduplicated shared functions (`conclave-client/src/config.rs`)
- `load_group_mapping()`, `save_group_mapping()`, and `generate_initial_key_packages()` were duplicated across `conclave-gui/src/app.rs`, `conclave-cli/src/main.rs`, and `conclave-cli/src/tui/commands.rs`.
- Moved all three to `conclave-client/src/config.rs` as public functions. Updated all call sites. The `save_group_mapping` in `conclave-cli/src/main.rs` was missing the Unix `0o600` permission setting — now fixed by using the centralized version.

#### M4. Section divider comments removed
- Removed 54 `// ──` section divider comments across 6 files per CLAUDE.md guideline: "Do not write organizational or comments that summarize the code."

#### M3. Abbreviated variable names renamed
- `req` → `request` in `conclave-server/src/api.rs` (50+ occurrences) and `conclave-client/src/api.rs`.
- `resp` → `response` in `conclave-client/src/api.rs`.
- `kp_data` → `key_package_data` in `conclave-server/src/api.rs`.
- `seq` → `sequence_number` in `conclave-server/src/api.rs` and `conclave-client/src/store.rs`.
- `mls_msg` → `mls_message` in `conclave-server/src/api.rs`.
- `gid` → `id` (tuple destructuring) in `conclave-server/src/api.rs`.
- `btn` → `room_button` in `conclave-gui/src/screen/dashboard.rs`.
- `col` → `messages_column`, `msg` → `message`, `row_el` → `row_element` in `conclave-gui/src/widget/message_view.rs`.

### Test Fixes

- Updated all 9 failing API integration tests to use `fake_key_package()` helper that prepends the MLS wire format header (version=1, wire_format=3) to test data, satisfying the new C5 validation.
- Updated `test_external_join_success` to store GroupInfo before the external join, satisfying the new C1 authorization check.
- All assertions updated to compare against the full key package bytes (header + payload).

## 2026-02-18: Startup Welcome Processing

### What Changed

Fixed a bug where users invited to a group while offline would see "group mapping not found" errors on startup. The root cause: neither CLI nor GUI processed pending welcomes at startup — welcome processing only happened via real-time SSE events or manual `/join`.

#### Problem

When user A invites user B (offline), the server adds B to the group and stores a pending welcome. When B starts their client, `load_rooms()` returns the new group (B is a member on the server), but B never processed the MLS welcome — so there's no group mapping or MLS state. The subsequent `fetch_missed_messages` fails because it looks up the group in `group_mapping` and finds nothing.

#### CLI Fix — `crates/conclave-cli/src/tui/mod.rs`

- Added `accept_pending_welcomes()` function that fetches pending welcomes from the server, processes each via `mls.join_group()`, updates the group mapping, uploads a replacement key package, and reloads rooms.
- Called at startup after `load_rooms()` but before `fetch_missed_messages()`.
- Called on SSE reconnect (`EsEvent::Open`) before fetching missed messages, covering the case where the user is invited while SSE is disconnected but the client is still running.

#### GUI Fix — `crates/conclave-gui/src/app.rs`

- Added `welcomes_processed: bool` flag to `Conclave` (mirrors existing `rooms_loaded` pattern).
- Fires `accept_welcomes()` task at startup alongside keygen and rooms tasks.
- Gated the initial missed message fetch in `handle_rooms_loaded()` on `welcomes_processed` being true. Whichever completes last (rooms or welcomes) triggers the fetch.
- `handle_welcomes_processed()` sets the flag and triggers the fetch if rooms are already loaded.
- SSE `Connected` handler now fires both `accept_welcomes()` and `fetch_all_missed_messages()` on reconnect.
- Extracted `fetch_all_missed_messages()` helper to avoid duplicating the fetch logic between `handle_rooms_loaded`, `handle_welcomes_processed`, and `SseUpdate::Connected`.
- Reset `welcomes_processed = false` in `perform_logout()`.

## 2026-02-17: Robust Message Synchronization

### What Changed

Improved MLS message decryption error handling and epoch retention to make message synchronization more robust when clients are offline or encounter desync.

#### Problem

When `decrypt_message()` failed (e.g., epoch data evicted after being offline too long), the error was silently swallowed as `DecryptedMessage::None` and `last_seen_seq` advanced past the undecryptable messages — making the loss permanent and invisible. The default mls-rs epoch retention of 3 was also too tight for real-world offline periods (3 commits = 3 member changes or key rotations would evict old epoch secrets).

#### DecryptedMessage::Failed Variant

- **`conclave-client/src/mls.rs`**: Added `DecryptedMessage::Failed(String)` variant. `decrypt_message()` now distinguishes between "can't process message from self" (harmless, returns `None`) and real decryption errors (returns `Failed(reason)`). Uses string matching on the `Display` output since `MlsError` is `#[non_exhaustive]` and Conclave wraps it as `Error::Mls(String)`.
- **`conclave-cli/src/tui/events.rs`**: `handle_new_message()` emits a system message on `Failed` with the failure reason and sequence number.
- **`conclave-cli/src/tui/mod.rs`**: `fetch_missed_messages()` emits a system message on `Failed` and continues processing subsequent messages.
- **`conclave-cli/src/main.rs`**: One-shot `messages` subcommand prints failures to stderr.
- **`conclave-gui/src/app.rs`**: `fetch_and_decrypt()` creates a system message (`is_system: true`) on `Failed` and continues.

#### Epoch Retention Increase

- **`conclave-client/src/mls.rs`**: Added `EPOCH_RETENTION` constant (16). Configured `with_max_epoch_retention(EPOCH_RETENTION)` on the SQLite group state storage in `build_client()`. This extends the window from 3 to 16 epochs, allowing offline catch-up across many more commits. RFC 9420 does not specify a recommended value — this is left to implementations.

#### GUI Fetch Deduplication

- **`conclave-gui/src/app.rs`**: Added `fetching_groups: HashSet<String>` field to `Conclave`. When an SSE `NewMessage` event arrives, the handler skips spawning a `fetch_and_decrypt` task if one is already in-flight for that group. The group ID is inserted before spawning and removed in `handle_messages_fetched()`. This prevents parallel tasks from racing on the same group's MLS state.

#### Design Decisions

- `last_seen_seq` is still advanced past failed messages. Permanently undecryptable messages (evicted epoch, corrupted state) cannot be retried, so blocking would cause infinite retry loops. Instead, the user is notified and can `/reset` if needed.
- String matching on error messages is pragmatic: `MlsError` is `#[non_exhaustive]`, and the "can't process message from self" case is the only expected error that should be silent.

## 2026-02-17: Configurable GUI Theme and XDG Compliance

### What Changed

Made the GUI theme fully configurable via a config file and separated config from data directories per the XDG Base Directory Specification.

#### Configurable Theme

- **`conclave-gui/src/theme/config.rs`** (new): `ThemeConfig` struct with 12 optional `#rrggbb` hex color fields (`background`, `surface`, `surface_bright`, `primary`, `text`, `text_secondary`, `text_muted`, `error`, `success`, `border`, `scrollbar`, `selection`). `HexColor` newtype with custom `Deserialize` for hex parsing. `ThemeConfig::load(config_dir)` reads the `[theme]` section from `config.toml`. `ThemeConfig::apply(base)` overlays user overrides onto the default Ferra palette — unspecified colors keep their defaults.
- **`conclave-gui/src/theme/mod.rs`**: Registered `pub mod config;`.
- **`conclave-gui/src/app.rs`**: `Conclave::new()` loads theme overrides from `<config_dir>/config.toml` and applies them to the default theme at startup.

#### XDG Config/Data Separation

Previously all files (config, session, MLS keys, messages) were stored in a single `data_dir` (`~/.local/share/conclave`). Now config and data are separated per XDG:

- **Config** (`~/.config/conclave/`): `config.toml` (user-edited settings and theme overrides). Respects `$XDG_CONFIG_HOME` and `$CONCLAVE_CONFIG_DIR`.
- **Data** (`~/.local/share/conclave/`): `session.toml`, `group_mapping.toml`, `message_history.db`, MLS cryptographic state. Unchanged.

Changes:
- **`conclave-client/src/config.rs`**: Added `config_dir: PathBuf` field to `ClientConfig` with `default_config_dir()` (mirrors `default_data_dir()` pattern: env var → XDG config dir → `.conclave` fallback). `ClientConfig::load()` now reads from `<config_dir>/config.toml` instead of `<data_dir>/config.toml`.
- **`conclave-cli/src/main.rs`**: Default config path changed from `conclave-cli.toml` in cwd to `ClientConfig::load()` (XDG config dir). The `-c` flag still accepts an explicit path.

#### Preset Themes

Three preset theme files shipped in `themes/` at the project root:
- **`themes/ferra.toml`**: Default Ferra dark palette (warm peach/brown tones).
- **`themes/navy.toml`**: Navy/gold palette derived via CIE L\*a\*b\* lightness matching from the Ferra structure. Deep navy (#1C2635) background, gold (#F0D074) primary, with all intermediate colors computed to preserve the original perceptual contrast hierarchy.
- **`themes/greyscale.toml`**: Pure greyscale (R=G=B) with L\*-matched grey values for each role. Error and success retain slight red/green tints for semantic distinction.

Users copy a preset's `[theme]` section into `~/.config/conclave/config.toml` to apply it.

## 2026-02-17: GUI Offline Message Fetch (WIP)

### What Changed

Addressing the race condition where the GUI fails to show messages sent while it was offline. Three changes in `conclave-gui/src/app.rs`:

1. **`rooms_loaded: bool` flag**: Added to `Conclave` struct. Gates SSE subscription in `subscription()` — SSE won't start until rooms are populated, preventing the `Connected` handler from iterating an empty room list.
2. **Initial fetch in `handle_rooms_loaded()`**: On the first load (startup catch-up), fetches missed messages for all rooms using `Task::batch`. Uses `was_loaded` pattern to skip on subsequent calls (from `/rooms`, invite, kick, etc.).
3. **Error-resilient `fetch_and_decrypt()`**: Changed per-message decryption from `??` (abort entire batch on first error) to `match`/`continue` (skip failed message, continue with rest). Matches the TUI's approach.
4. **Reset on logout**: `rooms_loaded` reset to `false` in `perform_logout()`.

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

**Client library** (`conclave-client`):
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

- **`conclave-client/src/mls.rs`**: Changed the `CIPHERSUITE` constant from `CipherSuite::CURVE25519_AES128` to `CipherSuite::CURVE448_CHACHA`.
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
- **`ClientConfig`**: `server_url` field removed. Remaining fields: `data_dir`, `config_dir`, and `accept_invalid_certs`.
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

- **`conclave-gui` crate**: New workspace member with `conclave-client` as shared dependency.
- **Screens**: Login (centered card with server URL, username, password, login/register toggle) and Dashboard (three-panel layout: sidebar with room list + unread counts, scrollable message area, chat input).
- **Theme**: Custom dark theme (Ferra-inspired palette) implementing `iced::theme::Base` with per-widget `Catalog` styles for buttons, containers, text, text inputs, and scrollables.
- **Subscriptions**: SSE event stream via `iced::Subscription::run_with()` keyed by auth token, plus a 1-second tick timer for connection status.
- **Async**: All API calls via `Task::perform()`. MLS crypto (sync) wrapped in `tokio::task::spawn_blocking`.
- **Commands**: All TUI `/` commands supported in the GUI text input.

#### Bug Fixes

- **Wrong server URL**: `LoginInfo` now carries the server URL from the login form so `ApiClient` connects to the correct server.
- **Room list not refreshing**: Group creation, invite, and kick operations now trigger automatic room list reload.
- **"group mapping not found"**: `create_group` now returns a `GroupCreated` message that updates `self.group_mapping` before switching to the new room.

## 2026-02-15: Shared Library Extraction (`conclave-client`)

### What Was Built

Extracted reusable client logic from `conclave-cli` into a new `conclave-client` library crate so both the CLI/TUI and GUI can share it.

#### Modules Moved

| Module | From | To |
|--------|------|----|
| `api.rs` | `conclave-cli/src/` | `conclave-client/src/api.rs` |
| `mls.rs` | `conclave-cli/src/` | `conclave-client/src/mls.rs` |
| `config.rs` | `conclave-cli/src/` | `conclave-client/src/config.rs` |
| `error.rs` | `conclave-cli/src/` | `conclave-client/src/error.rs` (removed `Terminal` variant) |
| `Room`, `DisplayMessage`, `ConnectionStatus` | `conclave-cli/src/tui/state.rs` | `conclave-client/src/state.rs` |
| `MessageStore` | `conclave-cli/src/tui/store.rs` | `conclave-client/src/store.rs` |
| `Command` enum + `parse()` | `conclave-cli/src/tui/commands.rs` | `conclave-client/src/command.rs` |

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

MLS assigns its own internal group IDs (opaque bytes). The server assigns integer IDs as group IDs. The client maintains a `group_mapping.toml` file (per user, under `data_dir/users/<username>/`) that maps server group IDs (integer as string keys, for TOML compatibility) to MLS group IDs (hex-encoded). This mapping is essential — without it, the client cannot locate the correct MLS group state for encryption/decryption.

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
