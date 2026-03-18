# Invite Endpoints

## Escrow Invite

Uploads pre-built MLS commit, Welcome, and GroupInfo for a pending invitation. This is phase 2 of the [escrow invite system](../flows/escrow-invite.md), following the key package consumption in `POST /api/v1/groups/{id}/invite`.

```
POST /api/v1/groups/{group_id}/escrow-invite
```

**Authentication**: Required. **Authorization**: Admin only.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group the invite is for. |

### Request Body — `EscrowInviteRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `invitee_id` | bytes | Yes | The user being invited (must not be empty). |
| `commit_message` | bytes | Yes | MLS commit that adds the invitee. Must not be empty. |
| `welcome_message` | bytes | Yes | MLS Welcome message for the invitee. Must not be empty. |
| `group_info` | bytes | Yes | Updated MLS GroupInfo. Must not be empty. |

### Response Body — `EscrowInviteResponse`

Empty message.

### Validation

- The invitee MUST exist (404 if not found).
- The invitee MUST NOT already be a group member (409 if already a member).
- There MUST NOT already be a pending invite for the same `(group_id, invitee_id)` pair (409 if duplicate).

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Invite escrowed successfully. |
| 400 Bad Request | `invitee_id` is empty, or any required field is empty. |
| 401 Unauthorized | Invalid token, not a member, or not an admin. |
| 404 Not Found | Invitee does not exist. |
| 409 Conflict | Invitee is already a group member, or a pending invite already exists for this group+invitee. |

### SSE Events

- **`InviteReceivedEvent`** — sent to the invitee only.

---

## List Pending Invites (User)

Lists all pending invites addressed to the authenticated user.

```
GET /api/v1/invites
```

**Authentication**: Required.

### Request Body

None.

### Response Body — `ListPendingInvitesResponse`

| Field | Type | Description |
|-------|------|-------------|
| `invites` | repeated `PendingInvite` | List of pending invites for the user. |

Each `PendingInvite`:

| Field | Type | Description |
|-------|------|-------------|
| `invite_id` | bytes | Unique invite identifier (UUID). |
| `group_id` | bytes | The group being invited to. |
| `group_name` | string | The group's name. |
| `group_alias` | string | The group's display alias (may be empty). |
| `inviter_username` | string | The inviting user's username. |
| `inviter_id` | bytes | The inviting user's ID (UUID). |
| `invitee_id` | bytes | The invited user's ID (the authenticated user). |
| `created_at` | uint64 | Unix timestamp of invite creation (seconds). |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success (may be empty). |
| 401 Unauthorized | Invalid or expired token. |

### SSE Events

None.

---

## Accept Invite

Accepts a pending invite. Atomically adds the user to the group and makes the escrowed Welcome available for processing.

```
POST /api/v1/invites/{invite_id}/accept
```

**Authentication**: Required. **Authorization**: The authenticated user MUST be the invite's `invitee_id`.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `invite_id` | string | The invite to accept. |

### Request Body

None.

### Response Body — `AcceptInviteResponse`

Empty message.

### Server Processing

Atomically within a single transaction:

1. Delete the pending invite.
2. Add the invitee to `group_members` with the `member` role.
3. Store the escrowed Welcome message as a pending welcome.
4. Store the escrowed commit as a group message (assigned the next sequence number).

### Client Follow-up

After accepting, the client MUST:

1. Fetch pending welcomes via `GET /api/v1/welcomes`.
2. Process the MLS Welcome message to join the group.
3. Acknowledge the welcome via `POST /api/v1/welcomes/{id}/accept`.
4. Upload replacement key packages to replenish consumed ones.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Invite accepted. |
| 401 Unauthorized | Invalid token, or the authenticated user is not the invitee. |
| 404 Not Found | Invite does not exist. |

### SSE Events

- **`WelcomeEvent`** — sent to the invitee.
- **`GroupUpdateEvent`** with `update_type: "commit"` — sent to existing group members (excludes the invitee).

---

## Decline Invite

Declines a pending invite. The escrowed materials are discarded.

```
POST /api/v1/invites/{invite_id}/decline
```

**Authentication**: Required. **Authorization**: The authenticated user MUST be the invite's `invitee_id`.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `invite_id` | string | The invite to decline. |

### Request Body

None.

### Response Body — `DeclineInviteResponse`

Empty message.

### Notes

When an invite is declined, the inviter's MLS group state already contains a phantom leaf for the invitee (added during the escrow commit). The `InviteDeclinedEvent` signals the inviter to perform a [key rotation](../flows/key-rotation.md) (empty commit) to evict this phantom leaf.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Invite declined. |
| 401 Unauthorized | Invalid token, or the authenticated user is not the invitee. |
| 404 Not Found | Invite does not exist. |

### SSE Events

- **`InviteDeclinedEvent`** — sent to the inviter only.

---

## List Group Pending Invites

Lists all pending invites for a specific group.

```
GET /api/v1/groups/{group_id}/invites
```

**Authentication**: Required. **Authorization**: Admin only.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to query. |

### Request Body

None.

### Response Body — `ListGroupPendingInvitesResponse`

| Field | Type | Description |
|-------|------|-------------|
| `invites` | repeated `PendingInvite` | List of pending invites for the group. |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success (may be empty). |
| 401 Unauthorized | Invalid token, not a member, or not an admin. |

### SSE Events

None.

---

## Cancel Invite

Cancels a pending invite for a user. Available to group admins.

```
POST /api/v1/groups/{group_id}/cancel-invite
```

**Authentication**: Required. **Authorization**: Admin only.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group the invite belongs to. |

### Request Body — `CancelInviteRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `invitee_id` | bytes | Yes | The user whose invite to cancel. |

### Response Body — `CancelInviteResponse`

Empty message.

### Notes

Cancelling an invite triggers the same phantom leaf cleanup as declining: the original inviter receives an `InviteDeclinedEvent` and SHOULD perform a key rotation.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Invite cancelled. |
| 401 Unauthorized | Invalid token, not a member, or not an admin. |
| 404 Not Found | No pending invite exists for this group + invitee. |

### SSE Events

- **`InviteCancelledEvent`** — sent to the invitee.
- **`InviteDeclinedEvent`** — sent to the original inviter (triggers phantom leaf cleanup).
