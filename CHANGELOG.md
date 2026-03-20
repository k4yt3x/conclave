# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Configurable `auth_header` setting for server and client config.
- Custom HTTP headers support (`[custom_headers]`) in client config.
- Proxy support (`proxy_url`) for HTTP, HTTPS, SOCKS5, and SOCKS5h.
- Machine-readable `ErrorCode` enum in `ErrorResponse` protobuf messages.
- Custom CA certificate support (`ca_cert_path`) for private/self-signed CAs.
- Zeroization of passwords, MLS signing keys, and session tokens in memory.

### Fixed

- Session reconnect validates the token before showing "Welcome back".
- Auth header misconfiguration no longer triggers auto-logout.
- TUI auto-logout now deletes the stale session file.

- Atomic file permissions on Unix for session and group mapping files.
- Proper error propagation for system clock errors in session expiry.

### Changed

- **BREAKING**: User and group IDs changed from integers to UUID v4.
- **BREAKING**: MLS credential identity changed from 8-byte i64 to 16-byte UUID.
- **BREAKING**: Protobuf wire format: all ID fields switched from `int64` to `bytes`.
- Default log level changed to `warn` in release, `info` in debug builds.
- Document AS/DS service roles in spec per RFC 9420 Section 3.

## [0.1.1] - 2026-03-02

### Changed

- Default session token TTL increased from 7 days to 30 days.
- Default invitation TTL increased from 7 days to 30 days.
- Session tokens now use sliding TTL: expiry is extended on every authenticated API call.
- Password change now requires current password verification and invalidates all sessions.
- `/register` command format changed to `/register <server> <username> [token]` with interactive password prompt.
- `/login` command format changed to `/login <server> <username>` with interactive password prompt.
- `/passwd` command now uses interactive masked password prompt (current, new, confirm).
- GUI registration now includes a confirm password field.
- Clients auto-logout on HTTP 401 instead of retrying indefinitely.

## [0.1.0] - 2026-03-02

### Added

- End-to-end encrypted group messaging built on MLS (RFC 9420) with CURVE448_CHACHA cipher suite (256-bit security).
- Interactive TUI client with IRC-style commands, Emacs keybinds, and command aliases.
- GUI client with three-panel chat layout, Elm-style architecture, and mouse-based text selection.
- Four built-in GUI theme presets (conclave, ferra, greyscale, navy) with full color customization.
- User registration and login with Argon2id password hashing and session token authentication.
- Configurable registration controls (public or invite-only with optional token).
- Account management: password change, display name aliases, identity reset, and account deletion (`/expunge`).
- Group lifecycle management: create, rename, configure message expiry, and delete (`/delete`).
- Role-based group admin system with promote/demote and admin-only operations.
- Two-phase escrow invite system with accept, decline, and cancel flows.
- Member management: invite, kick, leave, and external rejoin after identity reset.
- Message pagination with sequence-number-based fetching and missed message recovery.
- Message expiration with server-wide retention policy and per-group expiry settings.
- Delete-after-fetch watermark mode for immediate message cleanup.
- Real-time push notifications via Server-Sent Events (SSE).
- TOFU fingerprint verification with `/verify`, `/whois`, and `/trusted` commands.
- Key package management with FIFO consumption, last-resort fallback, and rate limiting.
- Desktop notifications with platform-specific hints and daemon availability detection.
- Single-instance lock to prevent concurrent client access to the same data directory.
- XDG Base Directory support for configuration and data storage.
- Server systemd service unit with security hardening.
- GitHub Actions CI/CD pipeline with Linux, macOS, and Windows release builds.
