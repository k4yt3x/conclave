# Identifier Conventions

## ID-First Referencing

Conclave uses an **ID-first referencing convention**: all API operations reference users and groups by their integer IDs (`user_id`, `group_id`). Human-readable names are used only at the user interface boundary.

## User Identifiers

Each user is assigned a unique integer `user_id` at registration. This ID is:

- Used in all API request bodies and path parameters for operations (invite, kick, promote, demote, send message, etc.).
- Embedded in MLS credentials as big-endian i64 bytes (8 bytes).
- Used in all SSE events to reference users.
- Stable for the lifetime of the account — it never changes.

### Usernames

Usernames are human-readable identifiers subject to the following constraints:

- 1–64 characters long.
- Must start with an ASCII alphanumeric character.
- May contain only ASCII letters, digits, and underscores.
- Must be unique across the server.

Usernames are used only in:

- `POST /api/v1/register` and `POST /api/v1/login` (authentication).
- `GET /api/v1/users/{username}` (name-to-ID resolution).

All other endpoints accept only integer IDs.

### Display Names (Aliases)

Users may set an optional `alias` (display name) which is shown in client UIs. Aliases are not unique and not used for identification in the protocol.

## Group Identifiers

Each group has two identifiers:

1. **Server group ID** (`group_id`): A unique integer assigned by the server at group creation. Used in all API paths and request bodies.
2. **MLS group ID** (`mls_group_id`): An opaque byte identifier assigned by the MLS layer, hex-encoded as a string. Set on the server during the first commit upload and used for MLS operations on the client.

### Group Names

Group names follow the same format rules as usernames (1–64 characters, ASCII alphanumeric start, letters/digits/underscores only). Group names are unique across the server.

Group names are used only in:

- `POST /api/v1/groups` (group creation).

### Group Aliases

Groups may have an optional `alias` (display name) set by admins via `PATCH /api/v1/groups/{id}`.

## Name Resolution

### Client-to-Server (Input)

When a user types a command referencing another user by name (e.g., `/invite alice`), the client MUST:

1. Resolve the username to a `user_id` via `GET /api/v1/users/{username}`.
2. Use the `user_id` in all subsequent API calls.

### Server-to-Client (Display)

For display purposes, clients SHOULD resolve user IDs to human-readable names using the following strategy:

1. **Local cache**: Check the in-memory member data populated by `ListGroupsResponse`, which includes `username`, `alias`, and `user_id` for all group members.
2. **Server lookup**: For cache misses (e.g., users who left the group), use `GET /api/v1/users/by-id/{user_id}`.
3. **Fallback**: If the lookup fails, display `user#<id>` (e.g., `user#42`).

### Batch Convenience

API responses that list members or groups (e.g., `ListGroupsResponse`, `PendingInvite`, `InviteReceivedEvent`) include human-readable names alongside IDs. This avoids N+1 lookup queries for display rendering. However, operational API endpoints MUST NOT accept names — only IDs.

### Exceptions

The following SSE events include human-readable names because the recipient cannot resolve them from their local cache:

- **`InviteReceivedEvent`**: Includes `group_name` and `group_alias` because the invitee is not yet a group member.
- **`PendingInvite`**: Includes `group_name`, `group_alias`, and `inviter_username` for the same reason.
