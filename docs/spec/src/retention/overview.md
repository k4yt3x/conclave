# Retention and Expiration

## Overview

Conclave provides two layers of message lifecycle control:

1. **Server-wide retention**: A global maximum message age configured by the server operator.
2. **Per-group expiration**: A stricter per-group policy set by group admins.

These two layers interact to determine the **effective expiry** for each group's messages.

## Server-Wide Retention

The server operator configures a global retention policy via the `message_retention` configuration field. This sets the maximum age for messages across all groups.

| Value | Behavior |
|-------|----------|
| `"-1"` (default) | Disabled — messages are kept indefinitely. |
| `"0"` | Delete-after-fetch — messages are deleted after all group members have fetched them. |
| Duration string (e.g., `"30d"`) | Messages older than this duration are periodically deleted. |

See [Duration Format](../appendices/duration-format.md) for the duration string syntax.

## Per-Group Expiration

Group admins can set a per-group message expiry via `PATCH /api/v1/groups/{id}` with the `message_expiry_seconds` field and `update_message_expiry: true`.

| Value | Behavior |
|-------|----------|
| `-1` (default) | Disabled — inherits the server-wide retention policy. |
| `0` | Delete-after-fetch — messages are deleted after all group members have fetched them. |
| Positive integer | Messages older than this many seconds are periodically deleted. |

### Constraint

When the server has a non-disabled retention policy (not `"-1"`), the per-group expiry MUST NOT exceed the server retention value. The server rejects such requests with HTTP 400.

## Effective Expiry Calculation

The effective expiry for a group is determined by combining the server-wide retention and per-group expiration:

| Server Retention | Group Expiry | Effective Expiry |
|-----------------|--------------|------------------|
| Disabled (`-1`) | Disabled (`-1`) | Disabled — messages kept indefinitely |
| Disabled (`-1`) | Positive `N` | `N` seconds |
| Disabled (`-1`) | `0` | Delete-after-fetch |
| Positive `S` | Disabled (`-1`) | `S` seconds |
| Positive `S` | Positive `N` | `min(S, N)` seconds (stricter wins) |
| `0` | Any | Delete-after-fetch (`0` always wins) |
| Any | `0` | Delete-after-fetch (`0` always wins) |

In summary:

- If either layer is `0` (delete-after-fetch), the effective expiry is `0`.
- If both layers have positive values, the smaller (stricter) value applies.
- If one layer is disabled (`-1`), the other layer's value is used.
- If both are disabled, messages are kept indefinitely.

## Delete-After-Fetch Mode

When a group's effective expiry is `0`, the server uses a **watermark-based deletion** strategy:

1. The server maintains a **fetch watermark** per member per group, tracking the highest sequence number each member has fetched.
2. When a member fetches messages via `GET /api/v1/groups/{id}/messages`, their watermark is updated.
3. When a member sends a message via `POST /api/v1/groups/{id}/messages`, their watermark is also updated (the sender has already "seen" their own message).
4. A message is deleted only after **ALL current group members** have fetched past its sequence number (i.e., the minimum watermark across all members exceeds the message's sequence number).

This ensures no group member misses a message, while still deleting messages as soon as possible after universal fetch.

## Background Cleanup

The server runs a periodic background task to enforce retention and expiration policies. The task runs at a configurable interval (default: 1 hour, configured via `cleanup_interval`).

The cleanup task performs:

1. **Time-based deletion**: For groups with a positive effective expiry, messages with `created_at` older than the expiry duration are deleted.
2. **Watermark-based deletion**: For groups with effective expiry `0`, messages whose sequence number falls below the minimum fetch watermark across all group members are deleted.
3. **Session cleanup**: Expired session tokens are deleted.
4. **Invite cleanup**: Pending invites older than the configured invite TTL (default: 7 days) are deleted.

## Client-Side Behavior

### Local Expiry Enforcement

After fetching messages, clients SHOULD also delete locally stored messages that exceed the group's effective expiry from their local message history. This ensures consistent behavior between server and client storage.

### Display Timer

For groups with active expiry policies, clients SHOULD run a periodic timer (e.g., every 1 second) to remove expired messages from the in-memory display. This provides real-time visual feedback as messages expire.

### Querying the Policy

Clients can query the retention policy for a specific group via `GET /api/v1/groups/{id}/retention`, which returns both the server-wide retention and the per-group expiry settings.
