# Group Lifecycle

## Group Creation

Creating a group is a two-step process:

1. **Server-side**: The client calls `POST /api/v1/groups` with a `group_name` (and optional `alias`). The server creates the group record and adds the creator as the sole member with the `admin` role. The server returns a `group_id`.

2. **MLS-side**: The client creates an MLS group locally (with no other members), producing a commit and GroupInfo. The client uploads these via `POST /api/v1/groups/{id}/commit`, which also sets the `mls_group_id` on the server.

After creation, the group has exactly one member (the creator). Additional members are added via the [escrow invite system](../flows/escrow-invite.md).

## Group ID Mapping

The server uses auto-increment integer IDs (`group_id`), while MLS uses opaque byte identifiers (`mls_group_id`, hex-encoded as strings). The server stores the `mls_group_id` in the groups table after the first commit upload and returns it in `ListGroupsResponse`.

Clients MUST maintain a mapping between server group IDs and MLS group IDs. This mapping is populated from the `ListGroupsResponse` on login or reconnection.

## Member Addition (Escrow Invite)

Adding members to an existing group uses a two-phase escrow system. See [Escrow Invite System](../flows/escrow-invite.md) for the complete flow.

In summary:

1. The admin fetches the target's key package via the invite endpoint.
2. The admin builds an MLS commit (adding the target) and Welcome message locally.
3. The admin uploads the commit, Welcome, and GroupInfo to the server's invite escrow.
4. The target receives a notification and can accept or decline.
5. On acceptance, the escrowed materials are finalized — the target processes the Welcome to join the group.

## Member Removal (Kick)

An admin can remove a member from a group:

1. The admin finds the target's MLS leaf index (by matching the `user_id` in the target's MLS `BasicCredential`).
2. The admin builds an MLS removal commit targeting that leaf index.
3. The admin uploads the commit and GroupInfo via `POST /api/v1/groups/{id}/remove`.
4. The server removes the member from the group membership table and stores the commit as a group message.
5. All remaining members (and the removed member) receive a `MemberRemovedEvent`.

See [Member Removal and Departure](../flows/member-removal.md) for the complete flow.

## Voluntary Departure (Leave)

A member can leave a group voluntarily:

1. The member builds an MLS self-removal commit.
2. The member uploads the commit and optional GroupInfo via `POST /api/v1/groups/{id}/leave`.
3. The server removes the member from the group membership table and stores the commit as a group message.
4. Remaining members receive a `MemberRemovedEvent`.
5. The departing member deletes their local MLS group state.

## Key Rotation

A group member (typically an admin) can rotate the group's key material by building an empty MLS commit (no proposals). This:

- Advances the group epoch.
- Rotates key material, providing forward secrecy.
- Is used to clean up phantom MLS leaves after a declined invite.

The commit is uploaded via `POST /api/v1/groups/{id}/commit`. See [Key Rotation](../flows/key-rotation.md).

## External Rejoin

After an account reset (where the user's MLS state is wiped and regenerated), the user must rejoin their groups using MLS external commits:

1. The user fetches the stored GroupInfo for each group via `GET /api/v1/groups/{id}/group-info`.
2. The user builds an MLS external commit, which joins the group with a new leaf. If the user knows their old leaf index, the external commit includes a self-removal proposal to remove the old leaf.
3. The user uploads the external commit via `POST /api/v1/groups/{id}/external-join`.
4. Other group members receive an `IdentityResetEvent`.

External join REQUIRES:

- The user MUST still be a member of the group on the server (group membership is not deleted during account reset).
- A stored GroupInfo MUST exist for the group (GroupInfo is stored during commit uploads, member removals, and leave operations by authorized members).

See [Account Reset and External Rejoin](../flows/account-reset.md) for the complete flow.

## GroupInfo Storage

The server stores the latest MLS GroupInfo blob for each group. GroupInfo is updated by:

- `POST /api/v1/groups/{id}/commit` — when `group_info` is provided.
- `POST /api/v1/groups/{id}/remove` — when `group_info` is provided.
- `POST /api/v1/groups/{id}/leave` — when `group_info` is provided.

GroupInfo is required for external joins. Clients SHOULD include GroupInfo in commit uploads to ensure it stays current.

## Admin Role Management

- **Promote**: An admin can promote a member to admin via `POST /api/v1/groups/{id}/promote`. This is a server-side metadata operation — it does not involve MLS.
- **Demote**: An admin can demote another admin to member via `POST /api/v1/groups/{id}/demote`. The server MUST reject demotion of the last remaining admin.
- **Initial admin**: The group creator is automatically assigned the `admin` role.
- **New member default**: Members added via the invite system receive the `member` role.
