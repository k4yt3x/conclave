# Validation Rules

This appendix documents all input validation rules enforced by the server.

## Username

| Property | Rule |
|----------|------|
| Length | 1–64 characters |
| First character | MUST be ASCII alphanumeric (`[a-zA-Z0-9]`) |
| Allowed characters | ASCII letters, digits, and underscores (`[a-zA-Z0-9_]`) |
| Uniqueness | MUST be unique across the server |

Regex: `^[a-zA-Z0-9][a-zA-Z0-9_]{0,63}$`

Error message: `"username must start with a letter or digit and contain only ASCII letters, digits, and underscores"`

## Group Name

Group names follow the **same rules** as usernames.

| Property | Rule |
|----------|------|
| Length | 1–64 characters |
| First character | MUST be ASCII alphanumeric (`[a-zA-Z0-9]`) |
| Allowed characters | ASCII letters, digits, and underscores (`[a-zA-Z0-9_]`) |
| Uniqueness | MUST be unique across the server |

Regex: `^[a-zA-Z0-9][a-zA-Z0-9_]{0,63}$`

## Password

| Property | Rule |
|----------|------|
| Minimum length | 8 characters |

No maximum length or character restrictions.

Error message: `"password must be at least 8 characters"`

## Alias (Display Name)

Used for both user aliases and group aliases.

| Property | Rule |
|----------|------|
| Maximum length | 64 characters |
| Forbidden characters | ASCII control characters: `0x00`–`0x1F` and `0x7F` |
| Unicode | Allowed |
| Uniqueness | NOT required |

Error messages:
- `"alias exceeds maximum length"` (if > 64 characters)
- `"must not contain ASCII control characters"` (if contains control characters)

## Registration Token

| Property | Rule |
|----------|------|
| Allowed characters | ASCII letters, digits, underscores, and hyphens (`[a-zA-Z0-9_-]`) |
| Validation timing | Validated at server config load time |
| Comparison | MUST use constant-time equality |

## Key Package Data

| Property | Rule |
|----------|------|
| Minimum size | 4 bytes |
| Maximum size | 16,384 bytes (16 KiB) |
| Bytes 0–1 | MUST be `0x00 0x01` (MLS version 1.0) |
| Bytes 2–3 | MUST be `0x00 0x05` (wire format `mls_key_package`, per RFC 9420 Section 6) |

## Message Expiry Seconds

| Property | Rule |
|----------|------|
| Allowed values | `-1` (disabled), `0` (delete-after-fetch), or any positive integer |
| Server constraint | When the server has a non-disabled retention policy (not `"-1"`), the group expiry MUST NOT exceed the server retention value |

## Message Fetch Limit

| Property | Rule |
|----------|------|
| Default | 100 messages per request |
| Maximum | 500 messages per request |

Values above 500 are capped to 500.

## Signing Key Fingerprint

| Property | Rule |
|----------|------|
| Format | Lowercase hexadecimal string |
| Length | 64 characters (SHA-256 output = 256 bits = 64 hex digits) |
| Validation | Not strictly validated on upload; stored as-is |

## Group Visibility

| Property | Rule |
|----------|------|
| Allowed values | `GROUP_VISIBILITY_PRIVATE` (1), `GROUP_VISIBILITY_PUBLIC` (2) |
| Default | `GROUP_VISIBILITY_PRIVATE` |
| Who can change | Group admins only (via `UpdateGroupRequest.visibility`) |

The `GROUP_VISIBILITY_UNSPECIFIED` (0) value in `UpdateGroupRequest` means no change to the current visibility. Setting a group to PUBLIC makes it discoverable via `GET /api/v1/groups/public` and joinable via `POST /api/v1/groups/{id}/join`.

## Key Package Count Limits

| Property | Rule |
|----------|------|
| Maximum regular packages per user | 10 |
| Maximum last-resort packages per user | 1 (new upload replaces previous) |
| Rate limit on consumption | 10 requests per minute per target user |
