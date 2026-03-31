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
| `mls_group_id` | string | Hex-encoded MLS group identifier. |
| `message_expiry_seconds` | int64 | Per-group message expiry (-1=disabled, 0=delete-after-fetch, >0=seconds). |
| `visibility` | `GroupVisibility` | PRIVATE (default) or PUBLIC. |

Each `GroupMember`:

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | bytes | Member's user ID (UUID). |
| `username` | string | Member's username. |
| `alias` | string | Member's display name (may be empty). |
| `role` | `GroupRole` | `GROUP_ROLE_ADMIN` or `GROUP_ROLE_MEMBER`. |
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
| `visibility` | `GroupVisibility` | No | New visibility setting. `UNSPECIFIED` (0) means no change. |

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

- **`GroupUpdateEvent`** with `update_type: GROUP_UPDATE_TYPE_GROUP_SETTINGS` — sent to all group members, **including the sender**.

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

## List Public Groups

Lists all groups with PUBLIC visibility. Available to any authenticated user.

```
GET /api/v1/groups/public?pattern={pattern}
```

**Authentication**: Required.

### Query Parameters

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `pattern` | string | No | Substring filter on `group_name`. Only groups whose name contains this string are returned. |

### Request Body

None.

### Response Body — `ListPublicGroupsResponse`

| Field | Type | Description |
|-------|------|-------------|
| `groups` | repeated `PublicGroupInfo` | List of public groups. |

Each `PublicGroupInfo`:

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | Server-assigned group ID (UUID). |
| `group_name` | string | Unique group name. |
| `alias` | string | Display name (may be empty). |
| `member_count` | uint32 | Number of members in the group. |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success. Returns an empty list if no public groups exist. |
| 401 Unauthorized | Invalid or expired token. |

### SSE Events

None.

---

## Join Public Group

Adds the caller as a member of a public group and returns the MLS GroupInfo needed to build an external commit. The caller must not already be a member.

```
POST /api/v1/groups/{group_id}/join
```

**Authentication**: Required.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The public group to join. |

### Request Body

None (empty body).

### Response Body — `GetGroupInfoResponse`

| Field | Type | Description |
|-------|------|-------------|
| `group_info` | bytes | MLS GroupInfo message for building an external commit. |

### Notes

This is step 1 of a two-step join flow:

1. **`POST /api/v1/groups/{group_id}/join`** — Server validates the group is public, adds the user as a member, and returns the MLS GroupInfo.
2. **`POST /api/v1/groups/{group_id}/external-join`** — Client builds an MLS external commit from the GroupInfo and submits it. The `external_join` handler detects this is a new joiner (no prior message history) and emits a `GroupUpdateEvent` instead of `IdentityResetEvent`.

The server adds the user as a member with the `member` role **before** the external commit is submitted. This maintains the server's authorization model while allowing the existing `external_join` endpoint's membership check to pass.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | User added as member. Returns MLS GroupInfo for external commit. |
| 400 Bad Request | No GroupInfo available for the group. |
| 403 Forbidden | Group is not public (`ERROR_CODE_GROUP_NOT_PUBLIC`). |
| 404 Not Found | Group does not exist. |
| 409 Conflict | User is already a member or has a pending invite. |

### SSE Events

None (SSE events are emitted by the subsequent `external_join` call).

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

## Ban Member

Bans a member from the group. Performs an MLS removal (like Remove Member) and additionally adds the user to the group's ban list, preventing them from rejoining via public join or invite acceptance. Any pending invites for the banned user in the group are also cancelled.

```
POST /api/v1/groups/{group_id}/ban
```

**Auth**: Required (admin only)

**Request Body**: `BanMemberRequest` (protobuf)

**Response**: `BanMemberResponse` (protobuf)

| Code | Condition |
|------|-----------|
| 200 OK | Member banned. |
| 400 Bad Request | Target is not a member. |
| 401 Unauthorized | Caller is not an admin. |
| 404 Not Found | User not found. |

### SSE Events

`MemberRemovedEvent` is broadcast to all remaining members and the banned user.

## Unban Member

Removes a user from the group's ban list, allowing them to rejoin.

```
POST /api/v1/groups/{group_id}/unban
```

**Auth**: Required (admin only)

**Request Body**: `UnbanMemberRequest` (protobuf)

**Response**: `UnbanMemberResponse` (protobuf)

| Code | Condition |
|------|-----------|
| 200 OK | User unbanned. |
| 400 Bad Request | User is not banned. |
| 401 Unauthorized | Caller is not an admin. |
| 404 Not Found | User not found. |

## List Banned Users

Lists all banned users for a group.

```
GET /api/v1/groups/{group_id}/banned
```

**Auth**: Required (admin only)

**Response**: `ListBannedUsersResponse` (protobuf)

| Code | Condition |
|------|-----------|
| 200 OK | Returns ban list. |
| 401 Unauthorized | Caller is not an admin. |
