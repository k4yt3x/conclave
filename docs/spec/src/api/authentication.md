# Authentication Endpoints

## Register

Creates a new user account.

```
POST /api/v1/register
```

**Authentication**: None (public endpoint).

### Request Body — `RegisterRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `username` | string | Yes | Unique username. 1–64 characters, must start with ASCII alphanumeric, only letters/digits/underscores. |
| `password` | string | Yes | Password. Minimum 8 characters. |
| `alias` | string | No | Display name. Max 64 characters, no ASCII control characters. |
| `registration_token` | string | No | Registration token for invite-only servers. Required when `registration_enabled` is `false`. |

### Response Body — `RegisterResponse`

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | int64 | The server-assigned unique user ID. |

### Status Codes

| Code | Condition |
|------|-----------|
| 201 Created | Registration successful. |
| 400 Bad Request | Invalid username format, password too short, alias too long or contains control characters. |
| 403 Forbidden | Registration is disabled, or the provided registration token is invalid. |
| 409 Conflict | Username already taken. |

### SSE Events

None.

---

## Login

Authenticates a user and returns a session token.

```
POST /api/v1/login
```

**Authentication**: None (public endpoint).

### Request Body — `LoginRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `username` | string | Yes | The user's username. |
| `password` | string | Yes | The user's password. |

### Response Body — `LoginResponse`

| Field | Type | Description |
|-------|------|-------------|
| `token` | string | Session token (256-bit random, hex-encoded, 64 characters). |
| `user_id` | int64 | The user's unique ID. |
| `username` | string | The user's username. |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Login successful. |
| 401 Unauthorized | Invalid username or password. |

### Notes

The server MUST perform timing-equalized password verification to prevent username enumeration. When the requested username does not exist, the server runs password verification against a dummy hash to ensure consistent response times.

### SSE Events

None.

---

## Logout

Revokes the current session token.

```
POST /api/v1/logout
```

**Authentication**: Required.

### Request Body

None.

### Response

HTTP **204 No Content** with no body.

### Status Codes

| Code | Condition |
|------|-----------|
| 204 No Content | Logout successful. Token revoked. |
| 401 Unauthorized | Invalid or expired token. |

### SSE Events

None.

---

## Get Current User

Returns the authenticated user's profile information.

```
GET /api/v1/me
```

**Authentication**: Required.

### Request Body

None.

### Response Body — `UserInfoResponse`

| Field | Type | Description |
|-------|------|-------------|
| `user_id` | int64 | The user's unique ID. |
| `username` | string | The user's username. |
| `alias` | string | The user's display name (may be empty). |
| `signing_key_fingerprint` | string | SHA-256 hex of the user's MLS signing public key (may be empty). |

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Success. |
| 401 Unauthorized | Invalid or expired token. |

### SSE Events

None.

---

## Update Profile

Updates the authenticated user's display name.

```
PATCH /api/v1/me
```

**Authentication**: Required.

### Request Body — `UpdateProfileRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `alias` | string | Yes | New display name. Max 64 characters, no ASCII control characters. Set to empty string to clear. |

### Response Body — `UpdateProfileResponse`

Empty message.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Profile updated. |
| 400 Bad Request | Alias too long or contains control characters. |
| 401 Unauthorized | Invalid or expired token. |

### SSE Events

- **`GroupUpdateEvent`** with `update_type: "member_profile"` — sent to all members of all groups the user belongs to, **including the sender**.

---

## Change Password

Changes the authenticated user's password.

```
POST /api/v1/change-password
```

**Authentication**: Required.

### Request Body — `ChangePasswordRequest`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `new_password` | string | Yes | The new password. Minimum 8 characters. |

### Response Body — `ChangePasswordResponse`

Empty message.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Password changed. |
| 400 Bad Request | New password too short. |
| 401 Unauthorized | Invalid or expired token. |

### Notes

Existing sessions remain valid after a password change. The server does NOT invalidate other sessions.

### SSE Events

None.

---

## Reset Account

Clears the user's server-side key packages, preparing for an MLS identity reset. The client is responsible for regenerating MLS state and rejoining groups via external commits.

```
POST /api/v1/reset-account
```

**Authentication**: Required.

### Request Body

None.

### Response Body — `ResetAccountResponse`

Empty message.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Key packages cleared. |
| 401 Unauthorized | Invalid or expired token. |

### Notes

This endpoint only deletes the user's key packages on the server. The client MUST then:

1. Wipe local MLS state (identity, signing key, group state database).
2. Generate a new MLS signing identity and key packages.
3. Upload the new key packages with the new fingerprint.
4. Rejoin each group via [external commit](../flows/account-reset.md).

Group membership on the server is NOT affected — the user remains a member of all their groups.

### SSE Events

None (the `IdentityResetEvent` is emitted later, when the user performs external joins to rejoin groups).
