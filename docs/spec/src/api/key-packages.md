# Key Package Endpoints

## Upload Key Packages

Uploads one or more MLS key packages for the authenticated user.

```
POST /api/v1/key-packages
```

**Authentication**: Required.

### Request Body — `UploadKeyPackageRequest`

The request supports two modes:

**Batch mode** (preferred):

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `entries` | repeated `KeyPackageEntry` | Yes | List of key packages to upload. |
| `signing_key_fingerprint` | string | No | SHA-256 hex of the user's MLS signing public key (64 characters). Stored for TOFU verification. |

Each `KeyPackageEntry`:

| Field | Type | Description |
|-------|------|-------------|
| `data` | bytes | Raw MLS key package bytes. |
| `is_last_resort` | bool | `true` for last-resort key packages, `false` for regular. |

**Legacy single-upload mode**:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `key_package_data` | bytes | Yes | A single key package (treated as a regular key package). |
| `signing_key_fingerprint` | string | No | SHA-256 hex of the signing public key. |

### Response Body — `UploadKeyPackageResponse`

Empty message.

### Validation

Each key package is validated:

- **Wire format**: The first 2 bytes MUST be `0x0001` (MLS version 1.0). The next 2 bytes MUST be `0x0005` (wire format `mls_key_package`). Minimum 4 bytes.
- **Size**: Each key package MUST be at most **16 KiB** (16,384 bytes).
- **Server cap**: The server stores at most **10 regular** key packages per user. Excess uploads replace the oldest packages.
- **Last-resort**: Uploading a last-resort package replaces any existing last-resort package for the user.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Key packages uploaded successfully. |
| 400 Bad Request | Key package fails wire format validation or exceeds size limit. |
| 401 Unauthorized | Invalid or expired token. |

### SSE Events

None.

---

## Fetch Key Package

Fetches (and consumes) a key package for the specified user. Used when inviting the user to a group.

```
GET /api/v1/key-packages/{user_id}
```

**Authentication**: Required.

### Path Parameters

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | string | The user whose key package to fetch (UUID hex string in URL). |

### Request Body

None.

### Response Body — `GetKeyPackageResponse`

| Field | Type | Description |
|-------|------|-------------|
| `key_package_data` | bytes | Raw MLS key package bytes. |

### Consumption Rules

1. The **oldest regular** key package is returned and **deleted** from the server (FIFO).
2. If no regular packages remain, the **last-resort** key package is returned but **NOT deleted**.
3. If no key packages of any kind exist, the server returns 404.

### Rate Limiting

This endpoint is rate-limited to **10 requests per minute per target `user_id`** to prevent key package exhaustion attacks.

### Status Codes

| Code | Condition |
|------|-----------|
| 200 OK | Key package returned (and consumed if regular). |
| 401 Unauthorized | Invalid or expired token. |
| 404 Not Found | No key packages available for this user. |
| 429 Too Many Requests | Rate limit exceeded. |

### SSE Events

None.
