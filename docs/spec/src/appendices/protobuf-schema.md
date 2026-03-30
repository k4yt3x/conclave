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
  bytes user_id = 1;
}

message LoginRequest {
  string username = 1;
  string password = 2;
}

message LoginResponse {
  string token = 1;
  bytes user_id = 2;
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
  string group_name = 2;
}

message CreateGroupResponse {
  bytes group_id = 1;
}

message GroupInfo {
  bytes group_id = 1;
  string alias = 2;
  repeated GroupMember members = 3;
  string group_name = 4;
  string mls_group_id = 5;                  // Hex-encoded MLS group ID
  int64 message_expiry_seconds = 6;         // -1=disabled, 0=fetch-then-delete, >0=seconds
  GroupVisibility visibility = 7;           // PRIVATE or PUBLIC
}

message GroupMember {
  bytes user_id = 1;
  string username = 2;
  string alias = 3;
  GroupRole role = 4;                       // MEMBER or ADMIN
  string signing_key_fingerprint = 5;       // SHA-256 hex of signing public key
}

message ListGroupsResponse {
  repeated GroupInfo groups = 1;
}

message PublicGroupInfo {
  bytes group_id = 1;
  string group_name = 2;
  string alias = 3;
  uint32 member_count = 4;
}

message ListPublicGroupsResponse {
  repeated PublicGroupInfo groups = 1;
}

message UpdateGroupRequest {
  string alias = 1;
  string group_name = 2;
  int64 message_expiry_seconds = 3;
  bool update_message_expiry = 4;           // Must be true to apply expiry field
  GroupVisibility visibility = 5;           // PRIVATE or PUBLIC
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
  bytes group_info = 2;                     // MLS GroupInfo for external commits
  string mls_group_id = 3;                  // Hex-encoded MLS group ID (set on creation)
}

message UploadCommitResponse {}
```

## Welcomes

```protobuf
message PendingWelcome {
  bytes group_id = 1;
  string group_alias = 2;
  bytes welcome_message = 3;               // Raw MLS Welcome bytes
  bytes welcome_id = 4;
}

message ListPendingWelcomesResponse {
  repeated PendingWelcome welcomes = 1;
}
```

## Invitations

```protobuf
message InviteToGroupRequest {
  repeated bytes user_ids = 1;
}

message MemberKeyPackage {
  bytes user_id = 1;
  bytes key_package_data = 2;
}

message InviteToGroupResponse {
  repeated MemberKeyPackage member_key_packages = 1;
}

message EscrowInviteRequest {
  bytes invitee_id = 1;
  bytes commit_message = 2;
  bytes welcome_message = 3;
  bytes group_info = 4;
}

message EscrowInviteResponse {}

message PendingInvite {
  bytes invite_id = 1;
  bytes group_id = 2;
  string group_name = 3;
  string group_alias = 4;
  string inviter_username = 5;
  uint64 created_at = 6;                   // Unix timestamp (seconds)
  bytes invitee_id = 7;
  bytes inviter_id = 8;
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
  bytes invitee_id = 1;
}

message CancelInviteResponse {}
```

## Admin Management

```protobuf
message PromoteMemberRequest {
  bytes user_id = 1;
}

message PromoteMemberResponse {}

message DemoteMemberRequest {
  bytes user_id = 1;
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
  bytes sender_id = 2;
  bytes mls_message = 3;                   // Encrypted MLS message (opaque blob)
  uint64 created_at = 4;                   // Unix timestamp (seconds)
}

message GetMessagesResponse {
  repeated StoredMessage messages = 1;
}
```

## Member Management

```protobuf
message RemoveMemberRequest {
  bytes user_id = 1;
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
  string current_password = 1;
  string new_password = 2;
}

message ChangePasswordResponse {}

message DeleteAccountRequest {
  string password = 1;
}

message DeleteAccountResponse {}

message DeleteGroupResponse {}
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
    GroupDeletedEvent group_deleted = 9;
  }
}

message NewMessageEvent {
  bytes group_id = 1;
  uint64 sequence_num = 2;
  bytes sender_id = 3;
}

message GroupUpdateEvent {
  bytes group_id = 1;
  GroupUpdateType update_type = 2;        // COMMIT, GROUP_SETTINGS, ROLE_CHANGE
}

message WelcomeEvent {
  bytes group_id = 1;
  string group_alias = 2;
}

message MemberRemovedEvent {
  bytes group_id = 1;
  bytes removed_user_id = 2;
}

message IdentityResetEvent {
  bytes group_id = 1;
  bytes user_id = 2;
}

message InviteReceivedEvent {
  bytes invite_id = 1;
  bytes group_id = 2;
  string group_name = 3;
  string group_alias = 4;
  bytes inviter_id = 5;
}

message InviteDeclinedEvent {
  bytes group_id = 1;
  bytes declined_user_id = 2;
}

message InviteCancelledEvent {
  bytes group_id = 1;
}

message GroupDeletedEvent {
  bytes group_id = 1;
}
```

## Enums

```protobuf
enum GroupVisibility {
  GROUP_VISIBILITY_UNSPECIFIED = 0;
  GROUP_VISIBILITY_PRIVATE = 1;
  GROUP_VISIBILITY_PUBLIC = 2;
}

enum GroupRole {
  GROUP_ROLE_UNSPECIFIED = 0;
  GROUP_ROLE_MEMBER = 1;
  GROUP_ROLE_ADMIN = 2;
}

enum GroupUpdateType {
  GROUP_UPDATE_TYPE_UNSPECIFIED = 0;
  GROUP_UPDATE_TYPE_COMMIT = 1;
  GROUP_UPDATE_TYPE_GROUP_SETTINGS = 2;
  GROUP_UPDATE_TYPE_ROLE_CHANGE = 3;
}
```

## Common

```protobuf
enum ErrorCode {
  ERROR_CODE_UNSPECIFIED = 0;

  // Input errors (100-199)
  ERROR_CODE_INPUT_BAD_REQUEST = 100;
  ERROR_CODE_INPUT_VALIDATION = 101;

  // Authentication errors (200-299)
  ERROR_CODE_AUTH_HEADER_MISSING = 200;
  ERROR_CODE_AUTH_HEADER_INVALID = 201;
  ERROR_CODE_AUTH_TOKEN_EXPIRED = 202;

  // Resource errors (300-399)
  ERROR_CODE_RESOURCE_NOT_FOUND = 300;
  ERROR_CODE_RESOURCE_CONFLICT = 301;
  ERROR_CODE_RESOURCE_FORBIDDEN = 302;

  // Group operation errors (400-499)
  ERROR_CODE_GROUP_NOT_MEMBER = 400;
  ERROR_CODE_GROUP_NOT_ADMIN = 401;
  ERROR_CODE_GROUP_NOT_PUBLIC = 402;
}

message ErrorResponse {
  string message = 1;                     // Human-readable error description
  ErrorCode error_code = 2;              // Machine-readable error code
}

message UserInfoResponse {
  bytes user_id = 1;
  string username = 2;
  string alias = 3;
  string signing_key_fingerprint = 4;     // SHA-256 hex of signing public key
}
```

