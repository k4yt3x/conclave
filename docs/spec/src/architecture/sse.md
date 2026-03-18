# Server-Sent Events

## Overview

Conclave uses [Server-Sent Events (SSE)](https://html.spec.whatwg.org/multipage/server-sent-events.html) for real-time server-to-client push notifications. When a relevant action occurs (new message, group update, invitation, etc.), the server pushes an event to all affected clients through their open SSE connections.

## SSE Endpoint

```
GET /api/v1/events
Authorization: Bearer <token>
```

This endpoint returns a persistent `text/event-stream` response. The server MUST send periodic keep-alive comments (`:` lines) to prevent connection timeouts.

## Event Wire Format

Each SSE event is a standard SSE `data:` line containing a **hex-encoded** serialized `ServerEvent` protobuf message:

```
data: 0a0c0801100a18012205616c696365
```

Clients MUST:
1. Read the `data` field from the SSE event.
2. Hex-decode the string to obtain raw bytes.
3. Deserialize the bytes as a `conclave.v1.ServerEvent` protobuf message.
4. Dispatch based on the `oneof event` variant.

## Event Targeting

Each event emitted by the server has a set of **target user IDs**. The SSE endpoint filters events so that each client only receives events addressed to their `user_id`. A client MUST NOT receive events targeted at other users.

## Sender Inclusion Rules

The server follows specific rules about whether the action's sender receives their own event:

- **Metadata operations** (profile update, group settings update, promote, demote): The sender IS included in the broadcast. All clients — including the sender's — receive the event and refresh their state.
- **MLS operations** (send message, upload commit, accept invite): The sender is EXCLUDED from the broadcast. The sender has already applied the MLS state change locally, so re-processing it would cause errors or duplicate state transitions.

## Event Types

The `ServerEvent` protobuf message uses a `oneof` to carry exactly one of the following event types:

### NewMessageEvent

Emitted when a new encrypted message is stored in a group.

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | The group the message was sent to |
| `sequence_num` | uint64 | The server-assigned sequence number |
| `sender_id` | bytes | The user who sent the message |

**Recipients**: All group members except the sender.

**Recommended client behavior**: Fetch new messages from the group starting after the client's last known sequence number.

### GroupUpdateEvent

Emitted when group state changes.

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | The affected group |
| `update_type` | string | The type of update (see below) |

**Update types**:

| Value | Trigger | Recipients |
|-------|---------|------------|
| `"commit"` | MLS commit uploaded (member add/remove via invite acceptance) | All members except the sender |
| `"member_profile"` | A member changed their profile (alias) | All co-members including the sender |
| `"group_settings"` | Group alias, name, or expiry changed | All members including the sender |
| `"role_change"` | A member was promoted or demoted | All members including the sender |

**Recommended client behavior**: Refresh the group's member list and metadata via `GET /api/v1/groups`.

### WelcomeEvent

Emitted when a user has a pending Welcome message to process (after accepting an invite).

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | The group the user was invited to |
| `group_alias` | string | The group's display alias |

**Recipients**: The invitee only.

**Recommended client behavior**: Fetch pending welcomes via `GET /api/v1/welcomes`, process MLS Welcome messages, and join the groups.

### MemberRemovedEvent

Emitted when a member is removed from or leaves a group.

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | The affected group |
| `removed_user_id` | bytes | The user who was removed or left |

**Recipients**: All remaining group members AND the removed user.

**Recommended client behavior**: If `removed_user_id` matches the client's own user ID, remove the group from local state. Otherwise, refresh the group's member list.

### IdentityResetEvent

Emitted when a member rejoins a group via external commit after an account reset.

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | The affected group |
| `user_id` | bytes | The user who reset their identity |

**Recipients**: All group members except the user who reset.

**Recommended client behavior**: Refresh the group state. Display a warning that the user's encryption keys have changed. Update the local TOFU fingerprint store if the fingerprint has changed.

### InviteReceivedEvent

Emitted when a pending invite is created for a user.

| Field | Type | Description |
|-------|------|-------------|
| `invite_id` | bytes | The invite's unique identifier |
| `group_id` | bytes | The group being invited to |
| `group_name` | string | The group's name |
| `group_alias` | string | The group's display alias |
| `inviter_id` | bytes | The user who sent the invite |

**Recipients**: The invitee only.

**Recommended client behavior**: Display the invitation with accept/decline options. The group name and alias are included because the invitee is not yet a member and cannot resolve them from their local cache.

### InviteDeclinedEvent

Emitted when an invitee declines a pending invite, or when an admin cancels a pending invite.

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | The group the invite was for |
| `declined_user_id` | bytes | The user who declined (or whose invite was cancelled) |

**Recipients**: The original inviter only.

**Recommended client behavior**: Perform a key rotation (empty MLS commit) to evict the phantom MLS leaf that was added when the invite was created. See [Key Rotation](../flows/key-rotation.md).

### InviteCancelledEvent

Emitted when an admin cancels a pending invite for a user.

| Field | Type | Description |
|-------|------|-------------|
| `group_id` | bytes | The group the invite was for |

**Recipients**: The invitee only.

**Recommended client behavior**: Remove the cancelled invite from the pending invites display.

## Lag Handling

If a client's SSE connection falls behind the server's event broadcast buffer, the server sends a transport-level lag notification:

```
event: lagged
data: 5
```

This is **not** a protobuf `ServerEvent` — it is a raw SSE event with the event type `"lagged"` and the data field containing the number of dropped events as a decimal string.

Clients SHOULD treat a `lagged` event as a signal to re-fetch all group state (via `GET /api/v1/groups` and relevant message endpoints) to ensure consistency.
