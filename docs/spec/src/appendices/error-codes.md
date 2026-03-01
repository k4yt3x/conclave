# Error Codes

All error responses use the `ErrorResponse` protobuf message with a `message` field containing a human-readable error description. The HTTP status code indicates the error category.

## Status Code Reference

### 400 Bad Request

Returned when the request contains invalid input.

| Condition | Example `message` |
|-----------|-------------------|
| Invalid username format | `"username must start with a letter or digit and contain only ASCII letters, digits, and underscores"` |
| Password too short | `"password must be at least 8 characters"` |
| Alias too long | `"alias exceeds maximum length"` |
| Alias contains control characters | `"must not contain ASCII control characters"` |
| Invalid group name format | Same as username validation |
| Missing required field | `"invitee_id is required"` |
| Empty required field | `"commit_message is required"` |
| Invalid message expiry value | `"message_expiry_seconds must be -1, 0, or positive"` |
| Group expiry exceeds server retention | `"group expiry cannot exceed server retention"` |
| Key package too large | `"key package exceeds maximum size"` |
| Key package wire format invalid | `"invalid key package wire format"` |
| Target not a group member | `"user is not a member of this group"` |
| Cannot demote last admin | `"cannot demote the last admin"` |
| No GroupInfo available for external join | `"no group info available"` |

### 401 Unauthorized

Returned when authentication or authorization fails.

| Condition | Context |
|-----------|---------|
| Missing `Authorization` header | Any authenticated endpoint |
| Invalid or expired token | Any authenticated endpoint |
| Invalid username or password | `POST /api/v1/login` |
| Not a member of the group | Group-scoped endpoints requiring membership |
| Not an admin of the group | Admin-only endpoints |
| Not the invitee for this invite | `POST /api/v1/invites/{id}/accept` or `decline` |

### 403 Forbidden

Returned when access is explicitly denied by server policy.

| Condition | Context |
|-----------|---------|
| Registration disabled | `POST /api/v1/register` when `registration_enabled` is `false` and no valid token provided |
| Invalid registration token | `POST /api/v1/register` with incorrect `registration_token` |

### 404 Not Found

Returned when the referenced resource does not exist.

| Condition | Context |
|-----------|---------|
| User not found | `GET /api/v1/users/{username}`, `GET /api/v1/users/by-id/{user_id}` |
| No key packages available | `GET /api/v1/key-packages/{user_id}` |
| Group not found | Group-scoped endpoints with invalid `group_id` |
| No GroupInfo stored | `GET /api/v1/groups/{id}/group-info` |
| Invite not found | `POST /api/v1/invites/{id}/accept`, `decline` |
| Welcome not found | `POST /api/v1/welcomes/{id}/accept` |
| No pending invite for group+invitee | `POST /api/v1/groups/{id}/cancel-invite` |
| Target user does not exist | `POST /api/v1/groups/{id}/invite`, `remove`, `promote`, `demote` |

### 409 Conflict

Returned when the request conflicts with existing state.

| Condition | Context |
|-----------|---------|
| Username already taken | `POST /api/v1/register` |
| Group name already taken | `POST /api/v1/groups` |
| User already a group member | `POST /api/v1/groups/{id}/invite`, `POST /api/v1/groups/{id}/escrow-invite` |
| User already an admin | `POST /api/v1/groups/{id}/promote` |
| Duplicate pending invite | `POST /api/v1/groups/{id}/escrow-invite` (same group+invitee) |

### 429 Too Many Requests

Returned when a rate limit is exceeded.

| Condition | Context |
|-----------|---------|
| Key package fetch rate exceeded | `GET /api/v1/key-packages/{user_id}` (10 req/min per target user) |

### 500 Internal Server Error

Returned for unexpected server-side failures.

| Condition | Notes |
|-----------|-------|
| Database errors | Connection failures, query errors, constraint violations |
| Password hashing failures | Argon2id computation errors |
| Protobuf encoding failures | Serialization errors |

The server MUST NOT expose internal details in the error message. The `message` field SHOULD be a generic string such as `"internal server error"`.

## Error Response Format

All errors use the same protobuf message:

```protobuf
message ErrorResponse {
  string message = 1;
}
```

Clients SHOULD display the `message` field to the user or include it in logs. Clients SHOULD also use the HTTP status code to determine the error category and take appropriate action (e.g., re-authenticate on 401, display validation feedback on 400).

## Notes

- The server uses HTTP **401** for both authentication failures (invalid token) and authorization failures (not a group member, not an admin). This is a simplification — implementations should be aware that 401 can indicate either condition.
- Error messages are informational and intended for human consumption. Clients SHOULD NOT parse error message strings programmatically — use the HTTP status code for control flow decisions.
