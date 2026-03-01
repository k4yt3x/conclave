# Protobuf Schema Reference

All wire format messages are defined in the `conclave.v1` protobuf package using proto3 syntax.

## Authentication

```protobuf
message RegisterRequest {
  string username = 1;
  string password = 2;
  string alias = 3;
  string registration_token = 4;
}

message RegisterResponse {
  int64 user_id = 1;
}

message LoginRequest {
  string username = 1;
  string password = 2;
}

message LoginResponse {
  string token = 1;
  int64 user_id = 2;
  string username = 3;
}
```

## Key Packages

```protobuf
message UploadKeyPackageRequest {
  bytes key_package_data = 1;                // Legacy single-upload
  repeated KeyPackageEntry entries = 2;      // Batch upload (preferred)
  string signing_key_fingerprint = 3;        // SHA-256 hex of signing public key
}

message KeyPackageEntry {
  bytes data = 1;                            // Raw MLS key package bytes
  bool is_last_resort = 2;                   // true for last-resort packages
}

message UploadKeyPackageResponse {}

message GetKeyPackageResponse {
  bytes key_package_data = 1;                // Raw MLS key package bytes
}
```

## Groups

```protobuf
message CreateGroupRequest {
  string alias = 1;
  // Field 2 reserved (was member_usernames).
  string group_name = 3;
}

message CreateGroupResponse {
  int64 group_id = 1;
  // Field 2 reserved (was member_key_packages).
}

message GroupInfo {
  int64 group_id = 1;
  string alias = 2;
  // Field 3 reserved (was creator_id).
  repeated GroupMember members = 4;
  uint64 created_at = 5;                    // Unix timestamp (seconds)
  string group_name = 6;
  string mls_group_id = 7;                  // Hex-encoded MLS group ID
  int64 message_expiry_seconds = 8;         // -1=disabled, 0=fetch-then-delete, >0=seconds
}

message GroupMember {
  int64 user_id = 1;
  string username = 2;
  string alias = 3;
  string role = 4;                          // "admin" or "member"
  string signing_key_fingerprint = 5;       // SHA-256 hex of signing public key
}

message ListGroupsResponse {
  repeated GroupInfo groups = 1;
}

message UpdateGroupRequest {
  string alias = 1;
  string group_name = 2;
  int64 message_expiry_seconds = 3;
  bool update_message_expiry = 4;           // Must be true to apply expiry field
}

message UpdateGroupResponse {}

message GetRetentionPolicyResponse {
  int64 server_retention_seconds = 1;       // -1=disabled, 0=fetch-then-delete, >0=seconds
  int64 group_expiry_seconds = 2;           // -1=disabled, 0=fetch-then-delete, >0=seconds
}
```

## Commits

```protobuf
message UploadCommitRequest {
  bytes commit_message = 1;
  // Field 2 reserved (was welcome_messages).
  bytes group_info = 3;                     // MLS GroupInfo for external commits
  string mls_group_id = 4;                  // Hex-encoded MLS group ID (set on creation)
}

message UploadCommitResponse {}
```

## Welcomes

```protobuf
message PendingWelcome {
  int64 group_id = 1;
  string group_alias = 2;
  bytes welcome_message = 3;               // Raw MLS Welcome bytes
  int64 welcome_id = 4;
}

message ListPendingWelcomesResponse {
  repeated PendingWelcome welcomes = 1;
}
```

## Invitations

```protobuf
message InviteToGroupRequest {
  repeated int64 user_ids = 1;
}

message InviteToGroupResponse {
  map<int64, bytes> member_key_packages = 1; // user_id → MLS key package
}

message EscrowInviteRequest {
  int64 invitee_id = 1;
  bytes commit_message = 2;
  bytes welcome_message = 3;
  bytes group_info = 4;
}

message EscrowInviteResponse {}

message PendingInvite {
  int64 invite_id = 1;
  int64 group_id = 2;
  string group_name = 3;
  string group_alias = 4;
  string inviter_username = 5;
  uint64 created_at = 6;                   // Unix timestamp (seconds)
  int64 invitee_id = 7;
  int64 inviter_id = 8;
}

message ListPendingInvitesResponse {
  repeated PendingInvite invites = 1;
}

message AcceptInviteResponse {}

message DeclineInviteResponse {}

message ListGroupPendingInvitesResponse {
  repeated PendingInvite invites = 1;
}

message CancelInviteRequest {
  int64 invitee_id = 1;
}

message CancelInviteResponse {}
```

## Admin Management

```protobuf
message PromoteMemberRequest {
  int64 user_id = 1;
}

message PromoteMemberResponse {}

message DemoteMemberRequest {
  int64 user_id = 1;
}

message DemoteMemberResponse {}

message ListAdminsResponse {
  repeated GroupMember admins = 1;
}
```

## Messages

```protobuf
message SendMessageRequest {
  bytes mls_message = 1;                   // Encrypted MLS application message
}

message SendMessageResponse {
  uint64 sequence_num = 1;                 // Server-assigned sequence number
}

message StoredMessage {
  uint64 sequence_num = 1;
  int64 sender_id = 2;
  // Field 3 reserved (was sender_username).
  bytes mls_message = 4;                   // Encrypted MLS message (opaque blob)
  uint64 created_at = 5;                   // Unix timestamp (seconds)
  // Field 6 reserved (was sender_alias).
}

message GetMessagesResponse {
  repeated StoredMessage messages = 1;
}
```

## Member Management

```protobuf
message RemoveMemberRequest {
  int64 user_id = 1;
  bytes commit_message = 2;               // MLS removal commit
  bytes group_info = 3;                    // Updated MLS GroupInfo
}

message RemoveMemberResponse {}

message LeaveGroupRequest {
  bytes commit_message = 1;               // MLS self-removal commit
  bytes group_info = 2;                    // Updated MLS GroupInfo
}

message LeaveGroupResponse {}
```

## External Commit

```protobuf
message GetGroupInfoResponse {
  bytes group_info = 1;                    // Raw MLS GroupInfo bytes
}

message ResetAccountResponse {}

message ExternalJoinRequest {
  bytes commit_message = 1;               // MLS external commit
  string mls_group_id = 2;                // Hex-encoded MLS group ID
}

message ExternalJoinResponse {}
```

## Profile Updates

```protobuf
message UpdateProfileRequest {
  string alias = 1;
}

message UpdateProfileResponse {}

message ChangePasswordRequest {
  // Field 1 reserved.
  string new_password = 2;
}

message ChangePasswordResponse {}
```

## SSE Events

```protobuf
message ServerEvent {
  oneof event {
    NewMessageEvent new_message = 1;
    GroupUpdateEvent group_update = 2;
    WelcomeEvent welcome = 3;
    MemberRemovedEvent member_removed = 4;
    IdentityResetEvent identity_reset = 5;
    InviteReceivedEvent invite_received = 6;
    InviteDeclinedEvent invite_declined = 7;
    InviteCancelledEvent invite_cancelled = 8;
  }
}

message NewMessageEvent {
  int64 group_id = 1;
  uint64 sequence_num = 2;
  int64 sender_id = 3;
}

message GroupUpdateEvent {
  int64 group_id = 1;
  string update_type = 2;                 // "commit", "member_profile", "group_settings", "role_change"
}

message WelcomeEvent {
  int64 group_id = 1;
  string group_alias = 2;
}

message MemberRemovedEvent {
  int64 group_id = 1;
  int64 removed_user_id = 2;
}

message IdentityResetEvent {
  int64 group_id = 1;
  int64 user_id = 2;
}

message InviteReceivedEvent {
  int64 invite_id = 1;
  int64 group_id = 2;
  string group_name = 3;
  string group_alias = 4;
  int64 inviter_id = 5;
}

message InviteDeclinedEvent {
  int64 group_id = 1;
  int64 declined_user_id = 2;
}

message InviteCancelledEvent {
  int64 group_id = 1;
}
```

## Common

```protobuf
message ErrorResponse {
  string message = 1;                     // Human-readable error description
}

message UserInfoResponse {
  int64 user_id = 1;
  string username = 2;
  string alias = 3;
  string signing_key_fingerprint = 4;     // SHA-256 hex of signing public key
}
```

## Reserved Fields

Several messages have gaps in field numbers due to removed fields. These field numbers are reserved and MUST NOT be reused with different semantics in future versions:

| Message | Field | Was |
|---------|-------|-----|
| `CreateGroupRequest` | 2 | `member_usernames` |
| `CreateGroupResponse` | 2 | `member_key_packages` |
| `GroupInfo` | 3 | `creator_id` |
| `UploadCommitRequest` | 2 | `welcome_messages` |
| `StoredMessage` | 3 | `sender_username` |
| `StoredMessage` | 6 | `sender_alias` |
| `ChangePasswordRequest` | 1 | (removed) |
