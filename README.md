# Conclave

> [!IMPORTANT]
> Conclave is still in early stages of development. The API is not yet stable and the codebase has not yet been audited.

A minimalistic, self-hosted, end-to-end encrypted group messaging system built on [MLS (RFC 9420)](https://www.rfc-editor.org/rfc/rfc9420.txt).

Full protocol specification and documentation: **[docs.conclave.im](https://docs.conclave.im)**

![Conclave GUI Screenshot](https://github.com/user-attachments/assets/49b94531-1a50-4a31-8f2d-bf0e4e19b6be)

## Overview

Conclave aims to make secure group communication accessible to everyone. All messages are encrypted client-side using IETF's standard MLS protocol. One binary, one SQLite database, zero external dependencies, five minutes to set up.

## Properties

- **5-minute setup**: single binary, no config required, no domain required
- **End-to-end encryption enforced**: MLS (RFC 9420) with forward secrecy and post-compromise security
- **Simple protocol**: single-server, no federation, minimal attack surface
- **Standard HTTP transport**: protobuf over HTTP/2 with SSE, compatible with reverse proxies and CDNs
- **Message expiration**: server-wide and per-room retention policies
- **Hardware security** (planned): TPM 2.0 support for key protection and database encryption at rest
- **Pure Rust reference implementation**: for better memory safety and performance

## Comparison

- **vs. Signal**
    - Centralized and not self-hostable; servers controlled by Signal Foundation
    - Requires a phone number linked to your identity for registration
- **vs. Matrix**
    - Federation adds protocol complexity, metadata leakage, attack surface, and is more error-prone
    - Olm library shipped with [known cryptographic vulnerabilities](https://soatok.blog/2024/08/14/security-issues-in-matrixs-olm-library/) (cache-timing side-channels, malleable signatures) for years
    - Poorly specified protocol leading to bugs (e.g., [canonical JSON issues reversing bans](https://neilalexander.dev/2024/06/05/canonical-json))
    - Heavy infrastructure requirements (Synapse, PostgreSQL)
- **vs. XMPP**
    - [E2E encryption (OMEMO) bolted on after the fact](https://soatok.blog/2024/08/04/against-xmppomemo/), with inconsistent client support
    - Encryption is not mandatory; unencrypted messaging is allowed by default
    - Federation shares the same concerns as Matrix

In contrast, Conclave:

- has a simple design that can be self-hosted with no federation;
- enforces end-to-end encryption for all conversations; and
- uses MLS (RFC 9420), an IETF standard designed for group encryption from the ground up with formal security proofs.

It should be noted that Conclave does not aim to fully replace any of these products, but to fill a niche while avoiding what this project considers undesirable design trade-offs.

## AI Use Declaration and Policy

AI tools were used to assist the design and implementation of this project. All design decisions were made by humans, and every change was reviewed and approved by a human maintainer. For contributors, AI use is accepted under the terms listed in the [AI Use Policy](https://github.com/k4yt3x/conclave/blob/master/CONTRIBUTING.md#ai-use-policy).

## License

This project is licensed under [AGPL-3.0-or-later](https://www.gnu.org/licenses/agpl-3.0.txt).\
Copyright 2026 K4YT3X.

![AGPLv3](https://www.gnu.org/graphics/agplv3-155x51.png)
