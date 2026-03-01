# Escrow Invite System

## Overview

Conclave uses a **two-phase escrow invite system** for all post-creation member additions. This system requires the target user to explicitly accept or decline an invitation before being added to a group. This prevents invite spam and gives users control over which groups they join.

The system works by having the inviter pre-build the MLS commit and Welcome message, then uploading them to the server for escrow. The target can inspect the invitation and choose to accept (triggering group join) or decline (discarding the escrowed materials and triggering phantom leaf cleanup).

## Full Invite Flow

### Phase 1: Key Package Consumption

```mermaid
sequenceDiagram
    participant A as Admin
    participant S as Server

    A->>S: POST /groups/{id}/invite<br>InviteToGroupRequest { user_ids: [target_id] }
    Note right of S: Validate user exists<br>Validate not already member<br>Consume key package (FIFO)
    S-->>A: 200 InviteToGroupResponse<br>{ member_key_packages: { target_id: kp_bytes } }
```

The admin requests key packages for the target users. The server validates each user, checks they are not already group members, and returns one consumed key package per user.

### Phase 2: Escrow

```mermaid
sequenceDiagram
    participant A as Admin
    participant S as Server
    participant T as Target

    Note over A: MLS: commit_builder().add_member(target_kp).build()<br>MLS: apply_pending_commit()<br>→ commit, welcome, group_info

    A->>S: POST /{id}/escrow-invite<br>EscrowInviteRequest { invitee_id, commit_message, welcome_message, group_info }
    Note right of S: Store in pending_invites
    S-->>A: 200 OK
    S--)T: SSE: InviteReceivedEvent<br>{ invite_id, group_id, group_name, inviter_id }
```

The admin builds the MLS commit (which adds the target as a new leaf) and the corresponding Welcome message. These are uploaded to the server's invite escrow along with the updated GroupInfo.

At this point, the admin's local MLS group state has already advanced — the target appears as a member in the admin's MLS tree. However, the target has not yet joined and the server has not yet added them to the group membership.

The target receives an `InviteReceivedEvent` via SSE.

### Phase 3a: Accept Path

```mermaid
sequenceDiagram
    participant T as Target
    participant S as Server
    participant A as Admin

    T->>S: POST /invites/{id}/accept
    Note right of S: Atomic transaction:<br>Delete pending_invite<br>Add to group_members<br>Store welcome → pending<br>Store commit as message
    S-->>T: 200 AcceptInviteResponse
    S--)T: SSE: WelcomeEvent (to target)
    S--)A: SSE: GroupUpdateEvent (to existing members)

    T->>S: GET /api/v1/welcomes
    S-->>T: ListPendingWelcomesResponse<br>{ welcomes: [{ welcome_id, group_id, welcome_message }] }

    Note over T: MLS: join_group(welcome_message)<br>→ mls_group_id

    T->>S: POST /welcomes/{id}/accept
    Note right of S: Delete pending welcome
    S-->>T: 204 No Content

    Note over T: Upload replacement key package
    T->>S: POST /api/v1/key-packages
    S-->>T: 200 OK
```

When the target accepts:

1. The server atomically: deletes the pending invite, adds the target to group members, stores the escrowed Welcome as a pending welcome, and stores the escrowed commit as a group message.
2. The target fetches and processes the Welcome message through the MLS layer to join the group.
3. The target acknowledges the Welcome and uploads a replacement key package.

### Phase 3b: Decline Path

```mermaid
sequenceDiagram
    participant T as Target
    participant S as Server
    participant A as Admin

    T->>S: POST /invites/{id}/decline
    Note right of S: Delete pending_invite
    S-->>T: 200 DeclineInviteResponse
    S--)A: SSE: InviteDeclinedEvent<br>{ group_id, declined_user_id }

    Note over A: Auto-rotate keys to evict phantom leaf

    A->>S: POST /{id}/commit<br>(empty commit = key rotation)
    S-->>A: 200 OK
```

When the target declines:

1. The server deletes the pending invite and the escrowed materials.
2. The inviter receives an `InviteDeclinedEvent` via SSE.
3. The inviter's client automatically performs a key rotation (empty MLS commit) to evict the phantom leaf that was added to the MLS tree during phase 2.

### Invite Cancellation

An admin can cancel a pending invite:

```mermaid
sequenceDiagram
    participant A as Admin
    participant S as Server
    participant T as Target

    A->>S: POST /{id}/cancel-invite<br>CancelInviteRequest { invitee_id }
    Note right of S: Delete pending_invite
    S-->>A: 200 OK
    S--)T: SSE: InviteCancelledEvent (to target)
    S--)A: SSE: InviteDeclinedEvent (to original inviter, triggers key rotation)
```

Cancellation triggers the same phantom leaf cleanup as declining: the original inviter receives an `InviteDeclinedEvent` and performs a key rotation.

## Constraints

- A user MUST NOT have more than one pending invite per group. The server enforces a unique constraint on `(group_id, invitee_id)`.
- Pending invites have a configurable TTL (default 7 days). Expired invites are cleaned up by the server's background task.
- The inviter MUST be an admin of the group.
- The invitee MUST exist and MUST NOT already be a group member.

## Phantom Leaf Problem

When the admin builds the MLS commit during phase 2, the MLS group state advances locally — the target appears as a new leaf in the MLS tree. If the target declines (or the invite is cancelled), this leaf is "phantom": it exists in the MLS tree but the user never actually joined.

The phantom leaf MUST be cleaned up via key rotation (an empty commit that advances the epoch). This is triggered automatically when the inviter's client receives an `InviteDeclinedEvent`.

If the phantom leaf is not cleaned up, subsequent MLS operations may fail or produce unexpected behavior, as the MLS tree contains a member who cannot participate in the group.
