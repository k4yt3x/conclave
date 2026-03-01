# Group Creation and Messaging

## Group Creation Flow

Creating a group involves a server-side operation followed by MLS group initialization.

```mermaid
sequenceDiagram
    participant C as Creator
    participant S as Server

    C->>S: POST /api/v1/groups<br>CreateGroupRequest { group_name, alias? }
    Note right of S: Create group record<br>Add creator as admin
    S-->>C: 201 CreateGroupResponse { group_id }

    Note over C: MLS: create_group()<br>MLS: commit_builder().build()<br>MLS: apply_pending_commit()<br>→ commit, group_info, mls_group_id

    C->>S: POST /groups/{id}/commit<br>UploadCommitRequest { commit_message, group_info, mls_group_id }
    Note right of S: Store commit as msg seq 1<br>Store GroupInfo<br>Set mls_group_id
    S-->>C: 200 OK

    Note over C: Store mapping: group_id → mls_group_id
```

### Steps

1. **Create on server**: The client sends the group name and optional alias. The server creates the group record and adds the creator as the sole member with the `admin` role.

2. **Initialize MLS group**: The client creates an MLS group locally. The initial MLS group has only the creator. The client builds an initial commit (which establishes the group's cryptographic state) and extracts the GroupInfo.

3. **Upload commit**: The client uploads the commit, GroupInfo, and the hex-encoded MLS group ID. The commit is stored as the first message (sequence number 1). The `mls_group_id` is recorded in the group's server record.

4. **Store mapping**: The client stores the mapping from the server's `group_id` to the MLS `mls_group_id` for future operations.

## Sending a Message

```mermaid
sequenceDiagram
    participant Sn as Sender
    participant S as Server
    participant R as Recipient

    Note over Sn: MLS: encrypt_application_message(plaintext)<br>→ ciphertext

    Sn->>S: POST /groups/{id}/messages<br>SendMessageRequest { mls_message: ciphertext }
    Note right of S: Store ciphertext blob<br>Assign sequence_num
    S-->>Sn: 200 SendMessageResponse { sequence_num }
    S--)R: SSE: NewMessageEvent<br>{ group_id, seq, sender_id }
```

### Steps

1. **Encrypt**: The client encrypts the plaintext message using MLS, producing an opaque ciphertext blob.

2. **Send**: The client sends the ciphertext to the server. The server stores it as an opaque blob and assigns a monotonically increasing sequence number.

3. **Notify**: The server broadcasts a `NewMessageEvent` to all group members except the sender, indicating that a new message is available.

## Receiving Messages

```mermaid
sequenceDiagram
    participant S as Server
    participant R as Recipient

    S--)R: SSE: NewMessageEvent<br>{ group_id, seq, sender_id }

    R->>S: GET /groups/{id}/messages?after={last_seen_seq}
    S-->>R: 200 GetMessagesResponse<br>{ messages: [{ seq, sender_id, mls_message, created_at }] }

    Note over R: MLS: process_incoming_message<br>→ plaintext<br>Resolve sender_id → display name<br>Display message
```

### Steps

1. **Receive notification**: The client receives a `NewMessageEvent` via SSE, indicating a new message is available in a group.

2. **Fetch messages**: The client fetches messages with sequence numbers after its last known sequence number using the `after` query parameter.

3. **Decrypt**: For each message, the client processes it through the MLS layer:
   - **Application messages** produce decrypted plaintext (chat messages).
   - **Commit messages** produce roster change information (members added/removed, key rotations).
   - **Failed decryption** produces an error reason (epoch evicted, key missing, etc.).

4. **Resolve sender**: The client maps the `sender_id` to a display name using its local member cache or the user lookup endpoint.

5. **Update tracking**: The client updates its last-seen sequence number to avoid re-fetching processed messages.

## Commit Messages vs. Application Messages

Both commits and application messages are stored in the same message table with sequential sequence numbers. The client distinguishes between them during MLS decryption:

- **Application messages**: `process_incoming_message()` returns decrypted plaintext bytes. These are user-visible chat messages.
- **Commit messages**: `process_incoming_message()` returns a commit result with information about roster changes (members added, members removed). Clients typically display these as system messages (e.g., "Alice joined the group", "Group keys updated").

An empty commit (no proposals, just epoch advancement) indicates a key rotation for forward secrecy.
