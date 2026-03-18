# Message Endpoints

## Upload Commit

Uploads an MLS commit message and optional GroupInfo for a group. Used for group creation, key rotation, and other MLS state changes that are not member additions or removals (those use the invite and remove endpoints).

```
POST /api/v1/groups/{group_id}/commit
```

**Authentication**: Required. **Authorization**: Group member.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group the commit belongs to. |

### Request Body — `UploadCommitRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `commit_message` | bytes | No | MLS commit message. Stored as a group message (assigned next sequence number) if provided. |
| `group_info` | bytes | No | Updated MLS GroupInfo. Stored for external commits if provided. |
| `mls_group_id` | string | No | Hex-encoded MLS group ID. Set on the server if not already set (typically on group creation). |

### Response Body — `UploadCommitResponse`

Empty message.

### Notes

All database operations (message storage, GroupInfo update, MLS group ID setting) are performed atomically within a single transaction. SSE notifications are sent only after the transaction commits.

If the `mls_group_id` is already set on the group, a new value is ignored (the MLS group ID is set once, on group creation).

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Commit uploaded. |
| 401 Unauthorized | Invalid token or not a group member. |

### SSE Events

- **`GroupUpdateEvent`** with `update_type: "commit"` — sent to all group members **except the sender** (if a commit message was provided).

---

## Send Message

Sends an encrypted MLS application message to a group.

```
POST /api/v1/groups/{group_id}/messages
```

**Authentication**: Required. **Authorization**: Group member.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to send the message to. |

### Request Body — `SendMessageRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `mls_message` | bytes | Yes | Encrypted MLS application message (opaque ciphertext). |

### Response Body — `SendMessageResponse`

| Field | Type | Description |
|-------|------|-------------|
| `sequence_num` | uint64 | The server-assigned sequence number for this message within the group. |

### Notes

The server stores the message as an opaque blob. It does not decrypt, validate, or interpret the MLS ciphertext. Messages are assigned a monotonically increasing sequence number within each group.

For groups with a delete-after-fetch policy (message expiry = 0), the server updates the sender's fetch watermark upon sending.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Message stored and sequence number assigned. |
| 401 Unauthorized | Invalid token or not a group member. |

### SSE Events

- **`NewMessageEvent`** — sent to all group members **except the sender**.

---

## Fetch Messages

Fetches encrypted messages from a group, paginated by sequence number.

```
GET /api/v1/groups/{group_id}/messages
```

**Authentication**: Required. **Authorization**: Group member.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `group_id` | string | The group to fetch messages from. |

### Query Parameters

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `after` | int64 | 0 | Fetch messages with sequence numbers strictly greater than this value. |
| `limit` | int64 | 100 | Maximum number of messages to return. Capped at **500**. |

### Request Body

None.

### Response Body — `GetMessagesResponse`

| Field | Type | Description |
|-------|------|-------------|
| `messages` | repeated `StoredMessage` | List of messages, ordered by sequence number (ascending). |

Each `StoredMessage`:

| Field | Type | Description |
|-------|------|-------------|
| `sequence_num` | uint64 | The message's sequence number within the group. |
| `sender_id` | bytes | The user ID of the sender (UUID). |
| `mls_message` | bytes | The encrypted MLS message (opaque ciphertext). |
| `created_at` | uint64 | Unix timestamp of when the server received the message (seconds). |

### Notes

Messages include both application messages (chat) and commit messages (MLS state changes like member additions and removals). The client distinguishes between them during MLS decryption — application messages produce plaintext, while commits produce roster change information.

For groups with a delete-after-fetch policy (message expiry = 0), the server updates the fetching user's watermark. Messages are deleted only after ALL group members have fetched past them.

### Pagination

To fetch all messages since a known point:

1. Set `after` to the highest sequence number the client has already processed.
2. Repeat with `after` set to the highest sequence number in the response until the response contains fewer messages than the limit.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success (may return empty list if no new messages). |
| 401 Unauthorized | Invalid token or not a group member. |

### SSE Events

None.
