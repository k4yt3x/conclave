# User Endpoints

## Look Up User by Username

Resolves a username to a user's profile information.

```
GET /api/v1/users/{username}
```

**Authentication**: Required.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `username` | string | The username to look up. |

### Request Body

None.

### Response Body — `UserInfoResponse`

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | bytes | The user's unique ID (UUID). |
| `username` | string | The user's username. |
| `alias` | string | The user's display name (may be empty). |
| `signing_key_fingerprint` | string | SHA-256 hex of the user's MLS signing public key (may be empty). |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | User found. |
| 401 Unauthorized | Invalid or expired token. |
| 404 Not Found | No user with that username exists. |

### SSE Events

None.

---

## Look Up User by ID

Resolves a user ID to a user's profile information.

```
GET /api/v1/users/by-id/{user_id}
```

**Authentication**: Required.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | string | The user ID to look up (UUID hex string in URL). |

### Request Body

None.

### Response Body — `UserInfoResponse`

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | bytes | The user's unique ID (UUID). |
| `username` | string | The user's username. |
| `alias` | string | The user's display name (may be empty). |
| `signing_key_fingerprint` | string | SHA-256 hex of the user's MLS signing public key (may be empty). |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | User found. |
| 401 Unauthorized | Invalid or expired token. |
| 404 Not Found | No user with that ID exists. |

### SSE Events

None.
