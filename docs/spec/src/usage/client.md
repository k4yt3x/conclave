# Client Configuration

Both the TUI (`conclave-cli`) and GUI (`conclave-gui`) read configuration from:

1. `$CONCLAVE_CONFIG_DIR/config.toml`
2. `$XDG_CONFIG_HOME/conclave/config.toml` (typically `~/.config/conclave/config.toml`)

All fields have sensible defaults and can be omitted. The client works without a config file.

## Command-Line Arguments

| Flag | Description |
|------|-------------|
| `-c`, `--config <path>` | Path to config file (overrides default search) |
| `-d`, `--data-dir <path>` | Path to data directory (overrides config file and env vars) |

Running `conclave-cli` with no subcommand launches the interactive TUI. Running `conclave-gui` launches the graphical interface.

## Configuration Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `data_dir` | string | `$CONCLAVE_DATA_DIR` or `$XDG_DATA_HOME/conclave` | Local data directory for SQLite databases, MLS keys, session state, and group mappings. |
| `accept_invalid_certs` | boolean | `false` | Accept invalid TLS certificates (e.g., self-signed). Only enable for development or when using Caddy's internal CA. |
| `show_verified_indicator` | boolean | `false` | Show verification indicators next to verified users and fully-verified rooms. When `false`, only unverified `[?]` and changed `[!]` indicators are shown. |
| `notifications` | string | `"Native"` | TUI-only. Notification method for new messages: `"Native"`, `"Bell"`, `"Both"`, or `"None"`. |

## Theme Customization

The GUI supports theme customization via the `[theme]` section. All fields are optional — unset fields keep the built-in defaults. Colors use `#rrggbb` hex format.

Preset themes are available in `assets/themes/`. Copy the `[theme]` section from a preset into your config file to use it.

Available presets: `conclave`, `ferra`, `greyscale`, `navy`.

## Full Reference

```toml
# Local data directory.
# Default: $CONCLAVE_DATA_DIR, or $XDG_DATA_HOME/conclave
#   (typically ~/.local/share/conclave)
#data_dir = "/home/user/.local/share/conclave"

# Accept invalid TLS certificates (e.g., self-signed). Default: false.
# Only enable this for development or testing environments.
#accept_invalid_certs = false

# Show verification indicators for verified users and fully-verified rooms.
# Default: false (hides verified indicators to reduce visual clutter).
#show_verified_indicator = false

# TUI-only: notification method for new messages.
# Possible values: "Native" (default), "Bell", "Both", "None".
#notifications = "Native"

# GUI theme overrides. All fields are optional; unset fields keep the
# built-in defaults. Colors use "#rrggbb" hex format.
# Theme presets are available in the assets/themes/ directory.
#[theme]
#background = "#2B292D"
#surface = "#242226"
#surface_bright = "#323034"
#title_bar = "#1E1C20"
#input_area = "#1E1C20"
#primary = "#FECDB2"
#text = "#FECDB2"
#text_secondary = "#AB8A79"
#text_muted = "#685650"
#error = "#E06B75"
#on_error = "#FFFFFF"
#warning = "#FFA07A"
#on_warning = "#2B292D"
#success = "#B1B695"
#border = "#4F474D"
#scrollbar = "#323034"
#selection = "#453D41"
```

## Data Directory Layout

After logging in, the client stores data under `data_dir`:

```
~/.local/share/conclave/
  conclave.lock             # Exclusive file lock (prevents multiple instances)
  session.toml              # Server URL, auth token, user ID
  users/<username>/
    mls.db                  # MLS key material (SQLite)
    message_history.db      # Message store and TOFU fingerprints (SQLite)
```

Only one Conclave client instance can run at a time per data directory. Launching a second instance will fail with an error. The lock is released automatically when the process exits.
