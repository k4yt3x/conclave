# Transport and Wire Format

## HTTP/2 Transport

All client-server communication uses HTTP/2. The server listens for connections on a configurable address and port.

### Transport Security

The server supports two transport modes:

1. **Plain HTTP**: When TLS is not configured, the server listens on plain HTTP. The default port is **8080**. This mode is intended for deployments behind a TLS-terminating reverse proxy (e.g., nginx, Cloudflare, Caddy).

2. **Native TLS**: When TLS certificate and key paths are configured, the server serves HTTPS directly. The default port is **8443**. The certificate and key MUST be in PEM format.

Clients MUST validate the server's TLS certificate when connecting over HTTPS. Implementations MAY provide an option to accept invalid certificates for development and testing purposes.

### URL Scheme

If a user specifies a server address without a URL scheme (e.g., `example.com:8443`), the client SHOULD automatically prepend `https://`.

## Wire Format

### Protocol Buffers

All request and response bodies use [Protocol Buffers (proto3)](https://protobuf.dev/programming-guides/proto3/) serialization. The protobuf package is `conclave.v1`.

- **Content-Type**: All requests and responses MUST use `Content-Type: application/x-protobuf`.
- **Encoding**: Bodies contain raw serialized protobuf bytes (not base64-encoded, not JSON).
- **Empty messages**: Protobuf message types with no fields (e.g., `UploadCommitResponse {}`) serialize to zero bytes. These are returned with the documented HTTP status code.

### Request Body Limits

The server MUST reject request bodies larger than **1 MiB** (1,048,576 bytes).

### Error Responses

All error responses use the `ErrorResponse` protobuf message:

```protobuf
message ErrorResponse {
  string message = 1;     // Human-readable error description
  ErrorCode error_code = 2; // Machine-readable error code
}
```

The `message` field contains a human-readable description for display or logging. The `error_code` field contains a machine-readable `ErrorCode` enum value for programmatic error handling. Error responses use the same `Content-Type: application/x-protobuf` encoding. See [Error Codes](../api/conventions.md#error-codes) for the full enum definition and code table.

The server MUST NOT expose internal implementation details (stack traces, database errors, file paths) in error messages returned to clients.

## Design Rationale

### Why Protobuf over HTTP Instead of gRPC

Conclave uses Protocol Buffers for message serialization without the gRPC transport layer. This provides schema-defined binary encoding with cross-language support while keeping the transport simple and proxy-friendly. Some CDN and reverse proxy services (e.g., Cloudflare) convert gRPC HTTP/2 to HTTP/1.1 gRPC-Web internally, which breaks bidirectional streaming and adds latency. Raw protobuf over standard HTTP/2 avoids these issues.

### Why SSE Instead of WebSockets or gRPC Streaming

Server-Sent Events (SSE) is a standard long-lived HTTP response that proxies through virtually all HTTP infrastructure without issues. It provides the server-to-client push channel needed for real-time notifications without requiring bidirectional streaming (client-to-server communication uses standard HTTP requests).
