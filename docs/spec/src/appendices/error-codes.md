# Error Codes

All error responses use the `ErrorResponse` protobuf message with a human-readable `message` field and a machine-readable `error_code` field. Clients MUST use `error_code` for programmatic decisions (e.g., auto-logout), not the message text. See [API Conventions — Error Codes](../api/conventions.md#error-codes) for the full `ErrorCode` enum definition and range conventions.

## Status Code Reference

### 400 Bad Request

Returned when the request contains invalid input.

| Condition | Error Code | Example `message` |
|-----------|------------|-------------------|
| Invalid username format | `ERROR_CODE_INPUT_VALIDATION` | `"username must start with a letter or digit..."` |
| Password too short | `ERROR_CODE_INPUT_VALIDATION` | `"password must be at least 8 characters"` |
| Alias too long | `ERROR_CODE_INPUT_VALIDATION` | `"alias exceeds maximum length"` |
| Alias contains control characters | `ERROR_CODE_INPUT_VALIDATION` | `"must not contain ASCII control characters"` |
| Invalid group name format | `ERROR_CODE_INPUT_VALIDATION` | Same as username validation |
| Missing required field | `ERROR_CODE_INPUT_BAD_REQUEST` | `"invitee_id is required"` |
| Empty required field | `ERROR_CODE_INPUT_BAD_REQUEST` | `"commit_message is required"` |
| Invalid message expiry value | `ERROR_CODE_INPUT_BAD_REQUEST` | `"message_expiry_seconds must be -1, 0, or positive"` |
| Group expiry exceeds server retention | `ERROR_CODE_INPUT_BAD_REQUEST` | `"group expiry cannot exceed server retention policy"` |
| Key package too large | `ERROR_CODE_INPUT_BAD_REQUEST` | `"key package exceeds maximum size"` |
| Key package wire format invalid | `ERROR_CODE_INPUT_BAD_REQUEST` | `"invalid key package wire format"` |
| Target not a group member | `ERROR_CODE_INPUT_BAD_REQUEST` | `"user is not a member of this group"` |
| Target not an admin | `ERROR_CODE_INPUT_BAD_REQUEST` | `"user is not an admin"` |
| Cannot demote last admin | `ERROR_CODE_INPUT_BAD_REQUEST` | `"cannot demote the last admin"` |
| No GroupInfo available for external join | `ERROR_CODE_INPUT_BAD_REQUEST` | `"no group info available"` |

### 401 Unauthorized

Returned when authentication or authorization fails.

| Condition | Error Code | Context |
|-----------|------------|---------|
| Missing auth header | `ERROR_CODE_AUTH_HEADER_MISSING` | Any authenticated endpoint |
| Invalid auth header format | `ERROR_CODE_AUTH_HEADER_INVALID` | Missing `Bearer` prefix on standard header |
| Invalid or expired token | `ERROR_CODE_AUTH_TOKEN_EXPIRED` | Any authenticated endpoint |
| Invalid username or password | `ERROR_CODE_AUTH_TOKEN_EXPIRED` | `POST /api/v1/login` |
| Not a member of the group | `ERROR_CODE_GROUP_NOT_MEMBER` | Group-scoped endpoints requiring membership |
| Not an admin of the group | `ERROR_CODE_GROUP_NOT_ADMIN` | Admin-only endpoints |
| Not the invitee for this invite | `ERROR_CODE_GROUP_NOT_MEMBER` | `POST /api/v1/invites/{id}/accept` or `decline` |

Clients SHOULD auto-logout only on `ERROR_CODE_AUTH_TOKEN_EXPIRED` (code 202). Auth header errors (200, 201) indicate a configuration mismatch that won't be resolved by re-logging in. Group membership/admin errors (400, 401) are operational and should be displayed as regular error messages.

### 403 Forbidden

Returned when access is explicitly denied by server policy.

| Scenario | Error Code | Endpoint(s) |
|----------|------------|-------------|
| Group is not public | `ERROR_CODE_GROUP_NOT_PUBLIC` | `POST /api/v1/groups/{id}/join` when group visibility is PRIVATE |
| User is banned | `ERROR_CODE_GROUP_BANNED` | `POST /api/v1/groups/{id}/join`, `POST /api/v1/groups/{id}/escrow-invite`, `POST /api/v1/invites/{id}/accept` |

| Condition | Error Code | Context |
|-----------|------------|---------|
| Registration disabled | `ERROR_CODE_RESOURCE_FORBIDDEN` | `POST /api/v1/register` when `registration_enabled` is `false` and no valid token provided |
| Invalid registration token | `ERROR_CODE_RESOURCE_FORBIDDEN` | `POST /api/v1/register` with incorrect `registration_token` |

### 404 Not Found

Returned when the referenced resource does not exist.

| Condition | Error Code | Context |
|-----------|------------|---------|
| User not found | `ERROR_CODE_RESOURCE_NOT_FOUND` | `GET /api/v1/users/{username}`, `GET /api/v1/users/by-id/{user_id}` |
| No key packages available | `ERROR_CODE_RESOURCE_NOT_FOUND` | `GET /api/v1/key-packages/{user_id}` |
| No GroupInfo stored | `ERROR_CODE_RESOURCE_NOT_FOUND` | `GET /api/v1/groups/{id}/group-info` |
| Invite not found | `ERROR_CODE_RESOURCE_NOT_FOUND` | `POST /api/v1/invites/{id}/accept`, `decline` |
| Welcome not found | `ERROR_CODE_RESOURCE_NOT_FOUND` | `POST /api/v1/welcomes/{id}/accept` |
| No pending invite for group+invitee | `ERROR_CODE_RESOURCE_NOT_FOUND` | `POST /api/v1/groups/{id}/cancel-invite` |
| Target user does not exist | `ERROR_CODE_RESOURCE_NOT_FOUND` | `POST /api/v1/groups/{id}/invite`, `remove`, `promote`, `demote` |

Non-existent groups return **401** (not 404) on group-scoped endpoints to prevent group existence probing.

### 409 Conflict

Returned when the request conflicts with existing state.

| Condition | Error Code | Context |
|-----------|------------|---------|
| Username already taken | `ERROR_CODE_RESOURCE_CONFLICT` | `POST /api/v1/register` |
| Group name already taken | `ERROR_CODE_RESOURCE_CONFLICT` | `POST /api/v1/groups` |
| User already a group member | `ERROR_CODE_RESOURCE_CONFLICT` | `POST /api/v1/groups/{id}/invite`, `escrow-invite` |
| User already an admin | `ERROR_CODE_RESOURCE_CONFLICT` | `POST /api/v1/groups/{id}/promote` |
| Duplicate pending invite | `ERROR_CODE_RESOURCE_CONFLICT` | `POST /api/v1/groups/{id}/escrow-invite` |

### 429 Too Many Requests

Returned when a rate limit is exceeded.

| Condition | Error Code | Context |
|-----------|------------|---------|
| Key package fetch rate exceeded | `ERROR_CODE_UNSPECIFIED` | `GET /api/v1/key-packages/{user_id}` (10 req/min per target user) |

### 500 Internal Server Error

Returned for unexpected server-side failures.

| Condition | Error Code | Notes |
|-----------|------------|-------|
| Database errors | `ERROR_CODE_UNSPECIFIED` | Connection failures, query errors, constraint violations |
| Password hashing failures | `ERROR_CODE_UNSPECIFIED` | Argon2id computation errors |
| Protobuf encoding failures | `ERROR_CODE_UNSPECIFIED` | Serialization errors |

The server MUST NOT expose internal details in the error message. The `message` field SHOULD be a generic string such as `"internal server error"`.

## Error Response Format

All errors use the `ErrorResponse` protobuf message:

```protobuf
message ErrorResponse {
  string message = 1;     // Human-readable error description
  ErrorCode error_code = 2; // Machine-readable error code
}
```

Clients SHOULD display the `message` field to the user or include it in logs. Clients MUST use the `error_code` field for programmatic control flow (e.g., distinguishing token expiry from auth header misconfiguration).

## Notes

- The server uses HTTP **401** for both authentication failures (invalid token) and authorization failures (not a group member, not an admin). The `error_code` field distinguishes these: `ERROR_CODE_AUTH_*` codes indicate authentication issues, while `ERROR_CODE_GROUP_*` codes indicate authorization issues.
- Error messages are informational and intended for human consumption. Clients MUST use `error_code` for programmatic decisions, not the message text.
