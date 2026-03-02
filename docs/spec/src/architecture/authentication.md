# Authentication

## Overview

Conclave uses a simple bearer token authentication model. Users register with a username and password, log in to receive an opaque session token, and include that token in subsequent requests.

## Registration

A client registers a new account by sending a `POST /api/v1/register` request with a username, password, and optional alias. See [Authentication Endpoints](../api/authentication.md) for the full endpoint specification.

### Registration Control

The server provides two configuration options to control registration access:

- **`registration_enabled`** (boolean, default `true`): When `true`, anyone can register and the token check is bypassed. When `false`, registration requires a valid registration token.
- **`registration_token`** (optional string): When `registration_enabled` is `false`, only requests providing this token can register. If no token is configured and registration is disabled, registration is entirely closed.

The registration token MUST contain only ASCII letters, digits, underscores, and hyphens (`[a-zA-Z0-9_-]`). Token comparison MUST use constant-time equality to prevent timing attacks.

The server MUST return HTTP 403 when registration is disabled or when the provided token is invalid.

## Password Hashing

Passwords MUST be hashed using **Argon2id** with a random salt before storage. The server MUST NOT store plaintext passwords.

## Login

A client logs in by sending a `POST /api/v1/login` request with a username and password. The server verifies the password against the stored Argon2id hash and, on success, generates a session token.

### Timing Attack Mitigation

To prevent username enumeration via timing analysis, the server MUST perform password verification even when the requested username does not exist. This is typically accomplished by verifying against a precomputed dummy Argon2id hash, ensuring both the valid-user and invalid-user code paths have equivalent computational profiles.

## Session Tokens

- Tokens MUST be 256-bit cryptographically random values, hex-encoded to 64 characters.
- Tokens MUST be generated using a cryptographically secure random number generator.
- Tokens MUST have a configurable time-to-live (TTL). The default TTL is **30 days** (2,592,000 seconds). Token expiry is extended on every authenticated API call (sliding window), so active sessions do not expire.
- The server SHOULD store a hash (e.g., SHA-256) of the token rather than the raw token value.

## Authenticated Requests

All endpoints except `POST /api/v1/register` and `POST /api/v1/login` require authentication.

Clients MUST include the session token in the `Authorization` header using the Bearer scheme:

```
Authorization: Bearer <token>
```

The server MUST validate the token against its session store and check the token's expiry. If the token is missing, invalid, or expired, the server MUST return HTTP 401.

## Logout

A client logs out by sending a `POST /api/v1/logout` request. The server MUST revoke the session token used in the request.

## Password Change

Authenticated users can change their password via `POST /api/v1/change-password`. The request requires verification of the current password. The server validates the new password (minimum 8 characters), hashes it with Argon2id, and updates the stored hash. All existing sessions (including the current one) MUST be invalidated after a password change. The user must log in again with the new password.
