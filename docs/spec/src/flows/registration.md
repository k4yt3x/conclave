# Registration and Login

## Registration Flow

Registration creates a new user account, establishes an MLS identity, and uploads initial key packages.

```mermaid
sequenceDiagram
    participant C as Client
    participant S as Server

    C->>S: POST /api/v1/register<br>RegisterRequest { username, password, alias?, token? }
    Note right of S: Validate username/password<br>Hash password (Argon2id)<br>Store user record
    S-->>C: 201 RegisterResponse { user_id }

    C->>S: POST /api/v1/login<br>LoginRequest { username, password }
    Note right of S: Verify password hash<br>Generate 256-bit token<br>Store session
    S-->>C: 200 LoginResponse { token, user_id, username }

    Note over C: Generate MLS signing key pair<br>Compute fingerprint = SHA-256(signing_public_key)<br>Generate 5 regular + 1 last-resort key packages

    C->>S: POST /api/v1/key-packages<br>UploadKeyPackageRequest { entries[6], signing_key_fingerprint }
    Note right of S: Validate wire format<br>Store key packages<br>Store fingerprint
    S-->>C: 200 OK

    Note over C: Save session locally:<br>server_url, token, user_id, username

    C->>S: GET /api/v1/events
    S-->>C: SSE stream established
```

### Steps

1. **Register**: The client sends the username, password, optional alias, and optional registration token. The server validates input, hashes the password, and creates the user record.

2. **Login**: The client immediately logs in after registration. The server verifies the password, generates a session token, and returns it with the user ID.

3. **Generate MLS identity**: The client generates an Ed448 signing key pair (part of the CURVE448_CHACHA cipher suite). The signing identity and secret key are persisted locally.

4. **Compute fingerprint**: The client computes `SHA-256(signing_public_key)` and formats it as a 64-character lowercase hex string.

5. **Upload key packages**: The client generates 5 regular and 1 last-resort key packages, then uploads them to the server along with the signing key fingerprint.

6. **Save session**: The client persists the server URL, token, user ID, and username locally for future requests.

7. **Connect SSE**: The client opens a persistent SSE connection for real-time event notifications.

## Login Flow

Login follows the same sequence as registration, except step 1 (register) is skipped:

```mermaid
sequenceDiagram
    participant C as Client
    participant S as Server

    C->>S: POST /api/v1/login<br>LoginRequest { username, password }
    Note right of S: Verify password hash<br>Generate token
    S-->>C: 200 LoginResponse { token, user_id, username }

    Note over C: Load or generate MLS identity<br>Compute fingerprint<br>Generate key packages

    C->>S: POST /api/v1/key-packages
    Note right of S: Store key packages<br>Store fingerprint
    S-->>C: 200 OK

    Note over C: Save session locally

    C->>S: GET /api/v1/events
    S-->>C: SSE stream established

    C->>S: GET /api/v1/groups
    S-->>C: 200 ListGroupsResponse

    Note over C: Build group ID mapping<br>Fetch missed messages per group
```

On login, clients SHOULD:

1. Load the existing MLS identity if available, or generate a new one.
2. Upload fresh key packages (replenishing any that were consumed while offline).
3. Fetch the group list to rebuild the server-group-ID to MLS-group-ID mapping.
4. Fetch any missed messages for each group (using the locally stored last-seen sequence number).
