# Event Stream

## SSE Endpoint

Opens a persistent Server-Sent Events stream for real-time notifications.

```
GET /api/v1/events
```

**Authentication**: Required.

### Response

- **Content-Type**: `text/event-stream`
- The connection remains open indefinitely.
- The server sends periodic keep-alive comments (`:` lines) to prevent timeouts.

### Wire Format

Each event is sent as an SSE `data:` line containing a hex-encoded serialized `ServerEvent` protobuf message:

```
data: <hex-encoded protobuf bytes>\n\n
```

### Protobuf Definition

```protobuf
message ServerEvent {
  oneof event {
    NewMessageEvent new_message = 1;
    GroupUpdateEvent group_update = 2;
    WelcomeEvent welcome = 3;
    MemberRemovedEvent member_removed = 4;
    IdentityResetEvent identity_reset = 5;
    InviteReceivedEvent invite_received = 6;
    InviteDeclinedEvent invite_declined = 7;
    InviteCancelledEvent invite_cancelled = 8;
    GroupDeletedEvent group_deleted = 9;
  }
}
```

### Event Type Reference

#### NewMessageEvent

A new encrypted message was stored in a group.

```protobuf
message NewMessageEvent {
  bytes group_id = 1;
  uint64 sequence_num = 2;
  bytes sender_id = 3;
}
```

**Recipients**: All group members except the sender.

#### GroupUpdateEvent

Group state changed (member roster, metadata, or roles).

```protobuf
message GroupUpdateEvent {
  bytes group_id = 1;
  string update_type = 2;
}
```

| `update_type` | Trigger | Recipients |
|---------------|---------|------------|
| `"commit"` | MLS commit stored (via invite acceptance) | All members except sender |
| `"member_profile"` | Member changed their profile alias | All co-members including sender |
| `"group_settings"` | Group alias/name/expiry changed | All members including sender |
| `"role_change"` | Member promoted or demoted | All members including sender |

#### WelcomeEvent

A pending Welcome message is ready for the user to process.

```protobuf
message WelcomeEvent {
  bytes group_id = 1;
  string group_alias = 2;
}
```

**Recipients**: The invitee only.

#### MemberRemovedEvent

A member was removed or left a group.

```protobuf
message MemberRemovedEvent {
  bytes group_id = 1;
  bytes removed_user_id = 2;
}
```

**Recipients**: All remaining group members AND the removed user.

#### IdentityResetEvent

A member rejoined a group with a new MLS identity via external commit.

```protobuf
message IdentityResetEvent {
  bytes group_id = 1;
  bytes user_id = 2;
}
```

**Recipients**: All group members except the user who reset.

#### InviteReceivedEvent

A pending invite was created for this user.

```protobuf
message InviteReceivedEvent {
  bytes invite_id = 1;
  bytes group_id = 2;
  string group_name = 3;
  string group_alias = 4;
  bytes inviter_id = 5;
}
```

**Recipients**: The invitee only.

#### InviteDeclinedEvent

An invitee declined a pending invite, or an admin cancelled an invite.

```protobuf
message InviteDeclinedEvent {
  bytes group_id = 1;
  bytes declined_user_id = 2;
}
```

**Recipients**: The original inviter only.

#### InviteCancelledEvent

An admin cancelled a pending invite for this user.

```protobuf
message InviteCancelledEvent {
  bytes group_id = 1;
}
```

**Recipients**: The invitee only.

#### GroupDeletedEvent

A group was permanently deleted by an admin.

```protobuf
message GroupDeletedEvent {
  bytes group_id = 1;
}
```

**Recipients**: All former group members (including the admin who deleted it).

### Lag Handling

If a client's SSE stream falls behind the server's broadcast buffer, the server sends a transport-level notification:

```
event: lagged
data: <number of dropped events>
```

This is NOT a protobuf `ServerEvent`. It is a raw SSE event with the event type `"lagged"` and the data field containing the number of dropped events as a decimal string.

Clients SHOULD treat a `lagged` event as a signal to perform a full state refresh (re-fetch group lists, messages, and pending invites/welcomes).

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | SSE stream established. |
| 401 Unauthorized | Invalid or expired token. |
