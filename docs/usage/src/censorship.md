# Censorship Circumvention

In censored environments, operators need to make a Conclave server indistinguishable from ordinary web traffic. This page covers deployment patterns that help resist active probing and server identification.

These techniques address the [Active Probing](/spec/security/threats.html#threat-active-probing--server-fingerprinting) and [Origin Server Discovery](/spec/security/threats.html#threat-origin-server-discovery) threats described in the protocol specification's threat model.

## CDN Fronting

Place the server behind a CDN such as Cloudflare. From the network's perspective, all traffic is standard HTTPS to the CDN's domain — indistinguishable from any other site behind the same CDN.

1. Point the domain's DNS to the CDN.
2. Configure the CDN to proxy traffic to the origin server.
3. **Firewall the origin** to only accept connections from the CDN's IP ranges. This prevents probers from bypassing the CDN and reaching the origin directly.
4. Clients connect using the CDN domain as the server URL.

## Tunnel Exposure

For servers behind NAT or without a public IP, tunnel services expose the server through an outbound connection. The origin has no open inbound ports and no public IP address.

**Cloudflare Tunnel:**

```bash
cloudflared tunnel --url http://localhost:8080
```

**ngrok:**

```bash
ngrok http 8080
```

Clients use the tunnel-provided URL (e.g., `https://abc123.ngrok-free.app`) as the server URL. The tunnel provider handles TLS termination and routing.

## Reverse Proxy Authentication

Deploy a reverse proxy (Caddy, Nginx, Apache) in front of Conclave that requires authentication before forwarding requests. Unauthenticated probes receive a generic 401 or 403 response, revealing nothing about the upstream service.

Clients use the [`[custom_headers]`](client.md#custom-headers) config section to send the required credentials on every request, including SSE connections.

### Caddy with Basic Auth

```
example.com {
    basicauth /* {
        user $2a$14$... # bcrypt hash
    }
    reverse_proxy localhost:8080
}
```

Client config (using custom `auth_header` so `Authorization` is free for the proxy):

```toml
auth_header = "X-Conclave-Token"

[custom_headers]
Authorization = "Basic dXNlcjpwYXNz"
```

The server must also set `auth_header = "X-Conclave-Token"` to match. Alternatively, if only the client authenticates with the proxy, the default `auth_header` can be kept — Conclave's per-request `Authorization: Bearer` token takes precedence over `[custom_headers]`.

### Nginx with Basic Auth

```nginx
server {
    listen 443 ssl;
    server_name example.com;

    auth_basic "Restricted";
    auth_basic_user_file /etc/nginx/.htpasswd;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
    }
}
```

### Custom Token Header

Instead of Basic Auth, the proxy can validate a custom header:

```nginx
server {
    listen 443 ssl;
    server_name example.com;

    location / {
        if ($http_x_access_token != "my-secret-token") {
            return 403;
        }
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
    }
}
```

Client config:

```toml
[custom_headers]
X-Access-Token = "my-secret-token"
```

## Secret Path Prefix

Serve Conclave under a non-obvious path prefix and proxy the default `/` to a benign website. Probers hitting the root see an ordinary site; only clients that know the prefix reach Conclave.

Clients include the prefix in the server URL at login — no additional client configuration is needed:

```
/login https://example.com/app-xyz123 username
```

### Caddy

```
example.com {
    handle /app-xyz123/* {
        uri strip_prefix /app-xyz123
        reverse_proxy localhost:8080
    }
    handle {
        reverse_proxy https://example-blog.com {
            header_up Host example-blog.com
        }
    }
}
```

### Nginx

```nginx
server {
    listen 443 ssl;
    server_name example.com;

    location /app-xyz123/ {
        proxy_pass http://127.0.0.1:8080/;
        proxy_set_header Host $host;
    }

    location / {
        proxy_pass https://example-blog.com;
        proxy_set_header Host example-blog.com;
    }
}
```

## Combining Techniques

These techniques compose for defense in depth:

- **CDN + proxy auth**: CDN hides the origin IP; proxy auth blocks unauthenticated probes that reach the CDN.
- **Tunnel + path prefix + decoy site**: Origin has no public IP; the tunnel URL serves a decoy at `/` and Conclave under a secret prefix.
- **CDN + path prefix + proxy auth**: Maximum protection — the origin is hidden, the path is secret, and authentication is required.

Choose the combination that fits your threat model and operational constraints.
