# Conclave Protocol Specification

**Version 0.1**

## Purpose

This document is the formal protocol specification for Conclave, a self-hosted, end-to-end encrypted group messaging system built on the Messaging Layer Security (MLS) protocol ([RFC 9420](https://www.rfc-editor.org/rfc/rfc9420.txt)). It defines the wire format, client-server API, protocol flows, and security properties necessary for any developer to implement a compatible Conclave server or client.

## What Is Conclave

Conclave is a private, encrypted chat system designed to make secure group communication accessible to everyone. It provides:

- **End-to-end encryption** via MLS — the server never sees plaintext message content.
- **Forward secrecy** and **post-compromise security** through MLS key ratcheting and epoch advancement.
- **Single-server architecture** — each server is an isolated community with no federation.
- **Simple deployment** — a single server binary, a single SQLite database file, and a single configuration file.

Conclave is a building block for third-party clients. Any application that implements this specification can interoperate with existing Conclave servers and clients.

## What Conclave Is Not

- **Not federated.** Each Conclave server is an isolated community. There is no server-to-server protocol. This is a deliberate choice to reduce protocol complexity, metadata leakage, and attack surface.
- **Not a user discovery service.** Users find each other out-of-band and connect to a known server.

## Design Principles

1. **Security**: MLS-based end-to-end encryption with no server-side access to plaintext. No compromises.
2. **Simplicity**: One code path for all messaging — both two-person conversations and group rooms use MLS groups. Minimal feature surface.
3. **Efficiency**: Binary wire format (Protocol Buffers). Compact storage (SQLite). Small binary footprint.
4. **Deployability**: Single static binary. Single SQLite file. Single config file. No external services.

## Scope

This specification covers:

- The **client-server HTTP API** — all endpoints, request/response formats, and error handling.
- The **wire format** — Protocol Buffers message definitions for all protocol messages.
- The **real-time event system** — Server-Sent Events (SSE) for push notifications.
- The **MLS integration** — how RFC 9420 primitives are used for encryption, key management, and group operations.
- **Protocol flows** — step-by-step sequences for registration, group creation, messaging, invitations, and more.
- **Security properties** — threat model, mitigations, and trust model.

This specification does not prescribe:

- Client-side storage formats or UI implementation details.
- Server-side database schema (the logical data model is described, but implementations may use any storage backend).
- Programming language or runtime requirements.

## Conventions

This specification uses the following conventions:

- **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are used as defined in [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119.txt).
- Protocol message field names are rendered in `monospace`.
- HTTP endpoints are written as `METHOD /path` (e.g., `POST /api/v1/register`).
- All protobuf types are in the `conclave.v1` package.
- MLS-specific terminology follows RFC 9420 definitions.

## Terminology

| Term | Definition |
|------|-----------|
| **Group** | An MLS group containing one or more members. All messaging occurs within groups. Also referred to as a "room" in client user interfaces. |
| **Member** | A user who belongs to a group and can send and receive encrypted messages within it. |
| **Admin** | A member with elevated privileges (invite, remove, promote, demote, update group settings). The group creator is the initial admin. |
| **Key package** | A pre-published MLS credential that allows other users to add someone to a group asynchronously (without the target being online). |
| **Last-resort key package** | A reusable key package that is never consumed, serving as a fallback when all regular key packages have been used. |
| **Epoch** | An MLS concept — the version counter for a group's key state. Epochs advance on each commit (member add/remove, key rotation). |
| **Commit** | An MLS operation that applies one or more proposals and advances the group epoch. |
| **Welcome** | An MLS message that allows a new member to join a group, containing the necessary key material and group state. |
| **External commit** | An MLS mechanism that allows a user to rejoin a group using only the group's public state (GroupInfo), without a Welcome message. Used for account reset. |
| **Escrow invite** | Conclave's two-phase invitation system where the inviter pre-builds the MLS commit and Welcome, and the invitee explicitly accepts or declines before joining. |
| **TOFU** | Trust On First Use — a trust model where the first observed signing key fingerprint for a user is assumed authentic and stored locally. Subsequent changes are flagged as warnings. |
| **Fingerprint** | The SHA-256 hash of a user's MLS signing public key, represented as a 64-character lowercase hexadecimal string. Used for identity verification. |
| **Sequence number** | A per-group, server-assigned monotonically increasing integer that orders messages within a group. |

## Normative References

- [RFC 9420 — The Messaging Layer Security (MLS) Protocol](https://www.rfc-editor.org/rfc/rfc9420.txt)
- [RFC 2119 — Key words for use in RFCs](https://www.rfc-editor.org/rfc/rfc2119.txt)
- [Protocol Buffers Language Guide (proto3)](https://protobuf.dev/programming-guides/proto3/)
