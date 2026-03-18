# Member Management Endpoints

## Invite to Group

Consumes key packages for the specified users, preparing for a group invitation. This is phase 1 of the [escrow invite system](../flows/escrow-invite.md).

```
POST /api/v1/groups/{group_id}/invite
```

**Authentication**: Required. **Authorization**: Admin only.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to invite users to. |

### Request Body — `InviteToGroupRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_ids` | repeated bytes | Yes | User IDs to invite. Must contain at least one ID. |

### Response Body — `InviteToGroupResponse`

| Field | Type | Description |
|-------|------|-------------|
| `member_key_packages` | repeated `MemberKeyPackage` | List of user ID to key package pairs. |

Each `MemberKeyPackage`:

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | bytes | The user's ID (UUID). |
| `key_package_data` | bytes | The consumed MLS key package bytes. |

### Notes

For each user ID in the request:
- The user MUST exist (404 if any user is not found).
- The user MUST NOT already be a group member (409 if already a member).
- A key package MUST be available for the user (404 if no key packages).
- If the user is the requester (self-invite), it is silently skipped.

After receiving the key packages, the client MUST build MLS commit and Welcome messages locally, then upload them via `POST /api/v1/groups/{id}/escrow-invite` for each invitee.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Key packages returned for all valid invitees. |
| 400 Bad Request | Empty `user_ids` list. |
| 401 Unauthorized | Invalid token, not a member, or not an admin. |
| 404 Not Found | A specified user does not exist, or no key package is available. |
| 409 Conflict | A specified user is already a group member. |

### SSE Events

None (events are emitted during the escrow phase).

---

## Remove Member

Removes a member from the group. The request includes the MLS removal commit.

```
POST /api/v1/groups/{group_id}/remove
```

**Authentication**: Required. **Authorization**: Admin only.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to remove the member from. |

### Request Body — `RemoveMemberRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | bytes | Yes | The user to remove. |
| `commit_message` | bytes | No | MLS commit for the removal. Stored as a group message if provided. |
| `group_info` | bytes | No | Updated MLS GroupInfo. Stored for external commits if provided. |

### Response Body — `RemoveMemberResponse`

Empty message.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Member removed. |
| 400 Bad Request | Target user is not a member of the group. |
| 401 Unauthorized | Invalid token, not a member, or not an admin. |
| 404 Not Found | Target user does not exist. |

### SSE Events

- **`MemberRemovedEvent`** — sent to all remaining group members AND the removed user.

---

## Leave Group

The authenticated user voluntarily leaves a group.

```
POST /api/v1/groups/{group_id}/leave
```

**Authentication**: Required. **Authorization**: Group member.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to leave. |

### Request Body — `LeaveGroupRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `commit_message` | bytes | No | MLS self-removal commit. Stored as a group message if provided. |
| `group_info` | bytes | No | Updated MLS GroupInfo. Stored for external commits if provided. |

### Response Body — `LeaveGroupResponse`

Empty message.

### Notes

After the server processes the request, the client SHOULD delete the local MLS group state for this group.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Successfully left the group. |
| 401 Unauthorized | Invalid token or not a member. |

### SSE Events

- **`MemberRemovedEvent`** — sent to remaining group members only (NOT the departing user).

---

## Promote Member

Promotes a group member to the admin role.

```
POST /api/v1/groups/{group_id}/promote
```

**Authentication**: Required. **Authorization**: Admin only.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group. |

### Request Body — `PromoteMemberRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | bytes | Yes | The member to promote. |

### Response Body — `PromoteMemberResponse`

Empty message.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Member promoted to admin. |
| 400 Bad Request | Target user is not a member of the group. |
| 401 Unauthorized | Invalid token, not a member, or not an admin. |
| 404 Not Found | Target user does not exist. |
| 409 Conflict | Target user is already an admin. |

### SSE Events

- **`GroupUpdateEvent`** with `update_type: "role_change"` — sent to all group members, **including the sender**.

---

## Demote Member

Demotes an admin to the member role.

```
POST /api/v1/groups/{group_id}/demote
```

**Authentication**: Required. **Authorization**: Admin only.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group. |

### Request Body — `DemoteMemberRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `user_id` | bytes | Yes | The admin to demote. |

### Response Body — `DemoteMemberResponse`

Empty message.

### Notes

The server MUST reject the demotion of the **last remaining admin** of a group. Every group must have at least one admin at all times.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Admin demoted to member. |
| 400 Bad Request | Target user is not an admin, or is the last admin of the group. |
| 401 Unauthorized | Invalid token, not a member, or not an admin. |
| 404 Not Found | Target user does not exist. |

### SSE Events

- **`GroupUpdateEvent`** with `update_type: "role_change"` — sent to all group members, **including the sender**.

---

## List Admins

Lists all members with the admin role in a group.

```
GET /api/v1/groups/{group_id}/admins
```

**Authentication**: Required. **Authorization**: Group member.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to query. |

### Request Body

None.

### Response Body — `ListAdminsResponse`

| Field | Type | Description |
|-------|------|-------------|
| `admins` | repeated `GroupMember` | List of group members with the `admin` role. |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success. |
| 401 Unauthorized | Invalid token or not a group member. |

### SSE Events

None.

---

## External Join

Rejoins a group via MLS external commit after an account reset. The user must still be a server-side group member.

```
POST /api/v1/groups/{group_id}/external-join
```

**Authentication**: Required. **Authorization**: Group member (must be an existing server-side member).

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to rejoin. |

### Request Body — `ExternalJoinRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `commit_message` | bytes | No | MLS external commit. Stored as a group message if provided. |
| `mls_group_id` | string | No | Hex-encoded MLS group ID. Set on the server if not already set. |

### Response Body — `ExternalJoinResponse`

Empty message.

### Prerequisites

- The user MUST be an existing server-side member of the group.
- A stored MLS GroupInfo MUST exist for the group (set by prior commit uploads, member removals, or leave operations).

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | External join successful. |
| 400 Bad Request | No GroupInfo available for the group. |
| 401 Unauthorized | Invalid token or not a group member. |
| 404 Not Found | Group does not exist. |

### SSE Events

- **`IdentityResetEvent`** — sent to all other group members (excludes the rejoining user) if a commit message was provided.
