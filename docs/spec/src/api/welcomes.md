# Welcome Endpoints

## List Pending Welcomes

Lists all pending MLS Welcome messages for the authenticated user. Welcomes become available after the user accepts a group invite.

```
GET /api/v1/welcomes
```

**Authentication**: Required.

### Request Body

None.

### Response Body — `ListPendingWelcomesResponse`

| Field | Type | Description |
|-------|------|-------------|
| `welcomes` | repeated `PendingWelcome` | List of pending Welcome messages. |

Each `PendingWelcome`:

| Field | Type | Description |
|-------|------|-------------|
| `welcome_id` | int64 | Unique welcome identifier. |
| `group_id` | int64 | The group this Welcome is for. |
| `group_alias` | string | The group's display alias (may be empty). |
| `welcome_message` | bytes | Raw MLS Welcome message bytes. |

### Notes

The client MUST process each Welcome message through the MLS layer to join the group before acknowledging it via `POST /api/v1/welcomes/{id}/accept`.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success (may be empty). |
| 401 Unauthorized | Invalid or expired token. |

### SSE Events

None.

---

## Accept Welcome

Acknowledges that the client has processed a pending Welcome message. The server deletes the Welcome after acknowledgment.

```
POST /api/v1/welcomes/{welcome_id}/accept
```

**Authentication**: Required.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `welcome_id` | int64 | The welcome to acknowledge. |

### Request Body

None.

### Response

HTTP **204 No Content** with no body.

### Notes

The client SHOULD call this endpoint only after successfully processing the MLS Welcome message locally (i.e., after `join_group()` succeeds). If the client crashes between processing the Welcome and calling this endpoint, the Welcome remains available for re-fetch and re-processing.

After processing all Welcomes, the client SHOULD upload replacement key packages to replenish any that were consumed during the invitation process.

### Status Codes

| Code | Condition |
|------|-----------|
| 204 No Content | Welcome acknowledged and deleted. |
| 401 Unauthorized | Invalid or expired token. |
| 404 Not Found | Welcome does not exist or does not belong to the authenticated user. |

### SSE Events

None.
