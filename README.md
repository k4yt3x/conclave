# Conclave

A minimalistic, self-hosted, end-to-end encrypted group messaging system built on [MLS](https://www.rfc-editor.org/rfc/rfc9420.txt).

> [!IMPORTANT]
> Conclave is in early stages of development. The API is not yet stable and the codebase has not yet been audited.

Full protocol specification and documentation: **[docs.conclave.im](https://docs.conclave.im)**

![Conclave GUI Screenshot](https://github.com/user-attachments/assets/49b94531-1a50-4a31-8f2d-bf0e4e19b6be)

## Overview

Conclave aims to make secure group communication accessible to everyone. All messages are encrypted client-side using IETF's standard MLS protocol. One binary, one SQLite database, zero external dependencies, five minutes to set up.

## Properties

- **5-minute setup**: single binary, no config required, no domain required
- **End-to-end encrypted**: MLS (RFC 9420) with forward secrecy and post-compromise security
- **Simple protocol**: single-server, no federation, minimal attack surface
- **Standard HTTP transport**: protobuf over HTTP/2 with SSE, compatible with reverse proxies and CDNs
- **Message expiration**: server-wide and per-room retention policies

## Comparison

- **vs. Signal**
    - Centralized and not self-hostable; servers controlled by Signal Foundation
    - Requires a phone number linked to your identity for registration
- **vs. Matrix**
    - Federation adds protocol complexity, metadata leakage, attack surface, and is more error-prone
    - Olm library shipped with [known cryptographic vulnerabilities](https://soatok.blog/2024/08/14/security-issues-in-matrixs-olm-library/) (cache-timing side-channels, malleable signatures) for years
    - Heavy infrastructure requirements (Synapse, PostgreSQL)
- **vs. XMPP**
    - [E2E encryption (OMEMO) bolted on after the fact](https://soatok.blog/2024/08/04/against-xmppomemo/), with inconsistent client support
    - Encryption is not mandatory; unencrypted messaging is allowed by default
    - Federation shares the same concerns as Matrix

In contrast, Conclave:

- has a simple design that can be self-hosted with no federation;
- enforces end-to-end encryption for all conversations; and
- uses MLS (RFC 9420), an IETF standard designed for group encryption from the ground up with formal security proofs.

## License

This project is licensed under [AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.txt).\
Copyright 2026 K4YT3X.

![AGPLv3](https://www.gnu.org/graphics/agplv3-155x51.png)
