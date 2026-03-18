# Summary

[Introduction](introduction.md)

# Architecture

- [Overview](architecture/overview.md)
- [Transport and Wire Format](architecture/transport.md)
- [Authentication](architecture/authentication.md)
- [Server-Sent Events](architecture/sse.md)
- [Identifier Conventions](architecture/id-referencing.md)

# MLS Integration

- [MLS Protocol Usage](mls/overview.md)
- [Key Packages](mls/key-packages.md)
- [Group Lifecycle](mls/group-lifecycle.md)
- [Identity and Credentials](mls/identity.md)
- [TOFU Fingerprint Verification](mls/tofu.md)

# Client-Server API

- [API Conventions](api/conventions.md)
- [Account Endpoints](api/accounts.md)
- [User Endpoints](api/users.md)
- [Key Package Endpoints](api/key-packages.md)
- [Group Endpoints](api/groups.md)
- [Member Management Endpoints](api/members.md)
- [Invite Endpoints](api/invites.md)
- [Welcome Endpoints](api/welcomes.md)
- [Message Endpoints](api/messages.md)
- [Event Stream](api/events.md)

# Protocol Flows

- [Registration and Login](flows/registration.md)
- [Group Creation and Messaging](flows/group-messaging.md)
- [Escrow Invite System](flows/escrow-invite.md)
- [Member Removal and Departure](flows/member-removal.md)
- [Account Reset and External Rejoin](flows/account-reset.md)
- [Key Rotation](flows/key-rotation.md)
- [Account Deletion](flows/account-deletion.md)
- [Group Deletion](flows/group-deletion.md)

# Message Retention

- [Retention and Expiration](retention/overview.md)

# Security Considerations

- [Security Properties](security/properties.md)
- [Threat Model and Mitigations](security/threats.md)

# Appendices

- [Protobuf Schema Reference](appendices/protobuf-schema.md)
- [Validation Rules](appendices/validation-rules.md)
- [Duration Format](appendices/duration-format.md)
- [Error Codes](appendices/error-codes.md)
