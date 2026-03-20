# API Conventions

## Base URL

All API endpoints are prefixed with `/api/v1/`. For example, given a server at `https://chat.example.com:8443`, the registration endpoint is:

```
https://chat.example.com:8443/api/v1/register
```

## Content Type

All request and response bodies MUST use `Content-Type: application/x-protobuf`. Bodies contain raw serialized Protocol Buffers bytes using the `conclave.v1` package.

Servers MUST reject requests with incorrect or missing content types for endpoints that expect a request body.

## Authentication

All endpoints except `POST /api/v1/register` and `POST /api/v1/login` require authentication. By default, clients MUST include the session token in the `Authorization` header:

```
Authorization: Bearer <token>
```

When the server and client are configured with a custom `auth_header` (e.g., `"X-Conclave-Token"`), the token is sent as a raw value without the `"Bearer "` prefix. See [Authentication](../architecture/authentication.md#configurable-auth-header) for details.

If the header is missing, the token is invalid, or the token has expired, the server MUST return **401 Unauthorized**.

## Request Body Limits

The server MUST reject request bodies larger than **1 MiB** (1,048,576 bytes).

## Path Parameters

Path parameters are denoted with `{name}` placeholders in endpoint paths. For example, in `/api/v1/groups/{group_id}/messages`, the `{group_id}` segment is replaced with the actual group ID.

## Query Parameters

Some endpoints accept query parameters for pagination or filtering. These are documented per-endpoint.

## HTTP Status Codes

The API uses the following HTTP status codes:

| Code | Meaning | Usage |
|------|---------|-------|
| **200 OK** | Request succeeded | Successful GET, POST, PATCH operations |
| **201 Created** | Resource created | Registration, group creation |
| **204 No Content** | Success with no response body | Logout, welcome acceptance |
| **400 Bad Request** | Validation error | Invalid input format, missing required fields, constraint violations |
| **401 Unauthorized** | Authentication or authorization failure | Missing/invalid/expired token, not a group member when membership is required |
| **403 Forbidden** | Access denied | Registration disabled, invalid registration token |
| **404 Not Found** | Resource does not exist | User, group, key package, invite, or welcome not found |
| **409 Conflict** | Duplicate resource | Username/group name taken, user already a member, duplicate invite |
| **500 Internal Server Error** | Server-side failure | Database errors, encoding failures (no internal details exposed) |

## Error Responses

All error responses use the `ErrorResponse` protobuf message:

```protobuf
message ErrorResponse {
  string message = 1;  // Human-readable error description
}
```

The `message` field contains a description suitable for displaying to the user or logging. The server MUST NOT include internal implementation details (stack traces, database errors, file paths) in error messages.

## Empty Response Bodies

Several response message types have no fields (e.g., `UploadCommitResponse {}`, `RemoveMemberResponse {}`). These serialize to zero bytes in protobuf. Endpoints returning these types use HTTP **200 OK** with an empty body, unless a different status code is documented (e.g., **204 No Content** for logout and welcome acceptance).

## Endpoint Documentation Format

Each endpoint in this specification is documented with:

- **HTTP method and path**
- **Authentication requirement** (public or authenticated)
- **Authorization requirement** (any authenticated user, group member, or group admin)
- **Request body** (protobuf message type and field descriptions)
- **Response body** (protobuf message type and field descriptions)
- **Query parameters** (if any)
- **Status codes** (success and error conditions)
- **SSE events emitted** (if any)
