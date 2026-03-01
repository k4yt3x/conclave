# Server Configuration

The server is configured via a TOML file. The server searches for configuration in the following order:

1. Path specified via `--config` (or `-c`) command-line flag.
2. `./conclave.toml` in the current working directory.
3. `/etc/conclave/config.toml`.
4. Built-in defaults (if no config file is found).

## Configuration Fields

### Network

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `listen_address` | string | `"0.0.0.0"` | IP address to bind to. |
| `listen_port` | integer | `8443` (TLS) or `8080` (plain HTTP) | Port to listen on. Default depends on whether TLS is configured. |

### Database

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `database_path` | string | `"conclave.db"` | Path to the SQLite database file. Created automatically if it does not exist. |

### Sessions

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `token_ttl_seconds` | integer | `604800` (7 days) | Session token lifetime in seconds. Tokens older than this are expired. |

### Invitations

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `invite_ttl_seconds` | integer | `604800` (7 days) | Pending invite lifetime in seconds. Expired invites are cleaned up by the background task. |

### Message Retention

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `message_retention` | string | `"-1"` | Global message retention policy. `"-1"` disables retention (keep forever). `"0"` enables delete-after-fetch. Duration format (e.g., `"30d"`) sets maximum message age. See [Duration Format](duration-format.md). |
| `cleanup_interval` | string | `"1h"` | Interval between background cleanup runs. Same duration format. |

### Registration Control

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `registration_enabled` | boolean | `true` | Whether public registration is open. When `false`, registration requires a valid token. |
| `registration_token` | string | (none) | Registration token for invite-only registration. Only checked when `registration_enabled` is `false`. Must contain only `[a-zA-Z0-9_-]`. |

### TLS

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `tls_cert_path` | string | (none) | Path to the TLS certificate file (PEM format). |
| `tls_key_path` | string | (none) | Path to the TLS private key file (PEM format). |

When both `tls_cert_path` and `tls_key_path` are set, the server serves HTTPS directly. When neither is set, the server serves plain HTTP (suitable for running behind a reverse proxy). Setting only one of the two is invalid.

## Example Configuration

### Minimal (Plain HTTP Behind Reverse Proxy)

```toml
listen_address = "127.0.0.1"
listen_port = 8080
database_path = "/var/lib/conclave/conclave.db"
```

### Native TLS

```toml
listen_address = "0.0.0.0"
listen_port = 8443
database_path = "/var/lib/conclave/conclave.db"
tls_cert_path = "/etc/conclave/cert.pem"
tls_key_path = "/etc/conclave/key.pem"
```

### Invite-Only with Message Retention

```toml
listen_address = "0.0.0.0"
database_path = "/var/lib/conclave/conclave.db"
registration_enabled = false
registration_token = "my-secret-invite-code"
message_retention = "30d"
cleanup_interval = "1h"
tls_cert_path = "/etc/conclave/cert.pem"
tls_key_path = "/etc/conclave/key.pem"
```

### Full Reference

```toml
# Network
listen_address = "0.0.0.0"
listen_port = 8443

# Database
database_path = "conclave.db"

# Sessions
token_ttl_seconds = 604800

# Invitations
invite_ttl_seconds = 604800

# Message retention
message_retention = "-1"
cleanup_interval = "1h"

# Registration
registration_enabled = true
# registration_token = "your-secret-token"

# TLS
# tls_cert_path = "/path/to/cert.pem"
# tls_key_path = "/path/to/key.pem"
```
