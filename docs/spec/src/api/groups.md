# Group Endpoints

## Create Group

Creates a new group with the authenticated user as the sole member and admin.

```
POST /api/v1/groups
```

**Authentication**: Required.

### Request Body — `CreateGroupRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `group_name` | string | Yes | Unique group name. 1–64 characters, must start with ASCII alphanumeric, only letters/digits/underscores. |
| `alias` | string | No | Display name. Max 64 characters, no ASCII control characters. |

### Response Body — `CreateGroupResponse`

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | The server-assigned unique group ID (UUID). |

### Notes

The creator is automatically added as a member with the `admin` role. No other members are added at creation time. Additional members are added via the [escrow invite system](invites.md).

After creating the group on the server, the client MUST:
1. Create an MLS group locally.
2. Upload the initial commit and GroupInfo via `POST /api/v1/groups/{id}/commit`, including the `mls_group_id`.

### Status Codes

| Code | Condition |
|------|-----------|
| 201 Created | Group created successfully. |
| 400 Bad Request | Invalid group name format, alias too long or contains control characters. |
| 401 Unauthorized | Invalid or expired token. |
| 409 Conflict | Group name already taken. |

### SSE Events

None.

---

## List Groups

Lists all groups the authenticated user is a member of, including member lists and metadata.

```
GET /api/v1/groups
```

**Authentication**: Required.

### Request Body

None.

### Response Body — `ListGroupsResponse`

| Field | Type | Description |
|-------|------|-------------|
| `groups` | repeated `GroupInfo` | List of groups the user belongs to. |

Each `GroupInfo`:

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | Server-assigned group ID (UUID). |
| `alias` | string | Display name (may be empty). |
| `group_name` | string | Unique group name. |
| `members` | repeated `GroupMember` | All members of the group. |
| `created_at` | uint64 | Unix timestamp of group creation (seconds). |
| `mls_group_id` | string | Hex-encoded MLS group identifier. |
| `message_expiry_seconds` | int64 | Per-group message expiry (-1=disabled, 0=delete-after-fetch, >0=seconds). |

Each `GroupMember`:

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | bytes | Member's user ID (UUID). |
| `username` | string | Member's username. |
| `alias` | string | Member's display name (may be empty). |
| `role` | string | Either `"admin"` or `"member"`. |
| `signing_key_fingerprint` | string | SHA-256 hex of the member's MLS signing public key (may be empty). |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success. |
| 401 Unauthorized | Invalid or expired token. |

### SSE Events

None.

---

## Update Group

Updates a group's alias, name, and/or message expiry settings.

```
PATCH /api/v1/groups/{group_id}
```

**Authentication**: Required. **Authorization**: Admin only.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to update. |

### Request Body — `UpdateGroupRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `alias` | string | No | New display name. Max 64 characters, no ASCII control characters. |
| `group_name` | string | No | New group name. Same validation rules as creation. |
| `message_expiry_seconds` | int64 | No | New message expiry value. Only applied when `update_message_expiry` is `true`. |
| `update_message_expiry` | bool | No | MUST be `true` for the `message_expiry_seconds` field to take effect. |

### Message Expiry Validation

When `update_message_expiry` is `true`:

- The value MUST be `-1` (disabled), `0` (delete-after-fetch), or a positive integer (seconds).
- If the server has a non-disabled retention policy (i.e., `message_retention` is not `"-1"`), the group expiry MUST NOT exceed the server retention value.

### Response Body — `UpdateGroupResponse`

Empty message.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Group updated. |
| 400 Bad Request | Invalid name/alias format, invalid expiry value, or expiry exceeds server retention. |
| 401 Unauthorized | Invalid token, not a member, or not an admin. |

### SSE Events

- **`GroupUpdateEvent`** with `update_type: "group_settings"` — sent to all group members, **including the sender**.

---

## Get Group Info

Returns the stored MLS GroupInfo blob for a group. Required for external commits (account reset / rejoin).

```
GET /api/v1/groups/{group_id}/group-info
```

**Authentication**: Required. **Authorization**: Group member.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group whose GroupInfo to fetch. |

### Request Body

None.

### Response Body — `GetGroupInfoResponse`

| Field | Type | Description |
|-------|------|-------------|
| `group_info` | bytes | Raw MLS GroupInfo bytes. |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | GroupInfo returned. |
| 401 Unauthorized | Invalid token or not a group member. |
| 404 Not Found | No GroupInfo has been stored for this group. |

### SSE Events

None.

---

## Get Retention Policy

Returns the server-wide retention policy and the group's per-group expiry setting.

```
GET /api/v1/groups/{group_id}/retention
```

**Authentication**: Required. **Authorization**: Group member.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to query. |

### Request Body

None.

### Response Body — `GetRetentionPolicyResponse`

| Field | Type | Description |
|-------|------|-------------|
| `server_retention_seconds` | int64 | Server-wide retention (-1=disabled, 0=delete-after-fetch, >0=seconds). |
| `group_expiry_seconds` | int64 | Per-group expiry (-1=disabled, 0=delete-after-fetch, >0=seconds). |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success. |
| 401 Unauthorized | Invalid token or not a group member. |

### SSE Events

None.

---

## Delete Group

Permanently deletes a group and all associated data.

```
POST /api/v1/groups/{group_id}/delete
```

**Authentication**: Required. **Authorization**: Group admin.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to delete. |

### Request Body

None.

### Response Body — `DeleteGroupResponse`

Empty message.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Group deleted. |
| 401 Unauthorized | Invalid token, not a group member, or not an admin. Non-existent groups also return 401 to prevent group existence probing. |

### Notes

This is an irreversible operation. The server:

1. Verifies the caller is a group admin.
2. Collects all group members for SSE notification.
3. Deletes the group row (`DELETE FROM groups WHERE id = ?`). CASCADE handles all dependent tables (group_members, messages, pending_invites, pending_welcomes, message_fetch_watermarks).
4. Broadcasts `GroupDeletedEvent` to all former members (including the caller).

All members' local MLS state for the group becomes orphaned. Clients clean up local MLS state upon receiving the `GroupDeletedEvent`.

### SSE Events

`GroupDeletedEvent` is broadcast to all former group members (including the caller).
