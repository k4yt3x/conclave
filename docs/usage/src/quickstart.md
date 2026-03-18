# Quick Start

This guide walks through setting up a Conclave server on a VPS with Caddy as a TLS reverse proxy, then connecting with a client.

## Prerequisites

- A VPS with a public IP address
- A domain name (optional)

## Download

Download the latest release binaries (`conclave-server`, `conclave-cli`, `conclave-gui`) from [GitHub Releases](https://github.com/k4yt3x/conclave/releases/latest).

## Server Setup

The server runs with sensible defaults and does not require a config file.

1. Install the `conclave-server` binary to `/usr/local/bin/conclave-server`.
2. Download the [systemd unit file](https://github.com/k4yt3x/conclave/blob/master/contrib/conclave-server.service) to `/etc/systemd/system/conclave-server.service`.
3. Create a `conclave` system user:
   ```bash
   sudo useradd -r -s /usr/sbin/nologin conclave
   ```
4. Start the service:
   ```bash
   sudo systemctl enable --now conclave-server
   ```

The server listens on `0.0.0.0:8080` (HTTP), stores data in `/var/lib/conclave/conclave.db`, and allows public registration. See [Server Configuration](server.md) for customization.

## TLS with Caddy

Conclave clients require HTTPS. The simplest approach is to run Caddy as a reverse proxy — it handles TLS certificates automatically.

1. Install Caddy ([install docs](https://caddyserver.com/docs/install)).
2. Create a Caddyfile. With a domain name:
   ```caddyfile
   example.conclave.im {
       reverse_proxy 127.0.0.1:8080
   }
   ```
   With an IP address (uses ACME short-lived certificates):
   ```caddyfile
   {
       default_sni 203.0.113.10
   }

   203.0.113.10 {
       reverse_proxy 127.0.0.1:8080

       tls {
           issuer acme {
               profile shortlived
           }
       }
   }
   ```
3. Enable and start Caddy:
   ```bash
   sudo systemctl enable --now caddy
   ```

## Connect and Chat

Launch `conclave-cli` (TUI) or `conclave-gui` (desktop). The commands below are for the TUI — the GUI provides equivalent functionality through its interface.

1. Register an account (you will be prompted for a password):
   ```
   /register example.conclave.im alice
   ```
2. Create a room:
   ```
   /create general
   ```
3. Invite other users (after they have registered):
   ```
   /invite bob,charlie
   ```
4. On the invited user's client, accept the invite:
   ```
   /accept
   ```
   This accepts all pending invites. To accept a specific invite, use `/accept <invite_id>`.
5. Send a message (type text without a `/` prefix):
   ```
   Hello, world!
   ```
6. View a user's signing key fingerprint:
   ```
   /whois bob
   ```
7. After confirming the fingerprint out-of-band, verify their identity:
   ```
   /verify bob a1b2c3d4 e5f6a7b8 c9d0e1f2 a3b4c5d6 e7f8a9b0 c1d2e3f4 a5b6c7d8 e9f0a1b2
   ```

Use `/help` to see all available commands.

## Next Steps

- `/help` — list all available commands.
- [Server Configuration](server.md) — require a token to register, set message retention, etc.
- [Client Configuration](client.md) — themes, notifications, data directories.
