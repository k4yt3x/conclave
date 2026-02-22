use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, patch, post};
use prost::Message;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::auth::{self, AuthUser};
use crate::db::validate_alias;
use crate::error::{Error, Result};
use crate::state::{AppState, SseEvent};

/// Build the axum router with all API routes.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Public endpoints
        .route("/api/v1/register", post(register))
        .route("/api/v1/login", post(login))
        // Authenticated endpoints
        .route("/api/v1/me", get(me).patch(update_profile))
        .route("/api/v1/key-packages", post(upload_key_package))
        .route("/api/v1/key-packages/{user_id}", get(get_key_package))
        .route("/api/v1/groups", post(create_group).get(list_groups))
        .route("/api/v1/groups/{group_id}", patch(update_group))
        .route("/api/v1/groups/{group_id}/invite", post(invite_to_group))
        .route("/api/v1/groups/{group_id}/commit", post(upload_commit))
        .route(
            "/api/v1/groups/{group_id}/messages",
            post(send_message).get(get_messages),
        )
        .route("/api/v1/welcomes", get(list_pending_welcomes))
        .route("/api/v1/welcomes/{welcome_id}/accept", post(accept_welcome))
        .route(
            "/api/v1/groups/{group_id}/remove",
            post(remove_group_member),
        )
        .route("/api/v1/groups/{group_id}/leave", post(leave_group))
        .route("/api/v1/groups/{group_id}/group-info", get(get_group_info))
        .route(
            "/api/v1/groups/{group_id}/external-join",
            post(external_join),
        )
        .route("/api/v1/reset-account", post(reset_account))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/events", get(sse_stream))
        .route("/api/v1/users/{username}", get(get_user_by_username))
        // Limit request body size to 1 MiB to prevent memory exhaustion.
        .layer(DefaultBodyLimit::max(1024 * 1024))
}

/// Helper to encode a protobuf message into a response body.
fn proto_response<M: Message>(status: StatusCode, msg: &M) -> impl IntoResponse + use<M> {
    let mut body = Vec::new();
    if let Err(error) = msg.encode(&mut body) {
        tracing::error!(%error, "failed to encode protobuf response");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/x-protobuf")],
            Vec::new(),
        );
    }
    (
        status,
        [(header::CONTENT_TYPE, "application/x-protobuf")],
        body,
    )
}

/// Helper to decode a protobuf request body.
fn decode_proto<M: Message + Default>(body: &Bytes) -> Result<M> {
    M::decode(body.as_ref()).map_err(Error::ProtobufDecode)
}

/// Validate that a key package blob is a structurally valid MLS KeyPackage
/// message per RFC 9420 Section 6. Checks the version (must be MLS 1.0 = 1)
/// and wire format (must be mls_key_package = 5). Full cryptographic
/// validation is left to the consuming client.
fn validate_key_package_wire_format(data: &[u8]) -> Result<()> {
    if data.len() < 4 {
        return Err(Error::BadRequest(
            "key package too short to be a valid MLS message".into(),
        ));
    }
    let version = u16::from_be_bytes([data[0], data[1]]);
    let wire_format = u16::from_be_bytes([data[2], data[3]]);
    if version != 1 {
        return Err(Error::BadRequest(format!(
            "unsupported MLS version {version}, expected 1"
        )));
    }
    // RFC 9420 Section 6: WireFormat mls_key_package = 5
    if wire_format != 5 {
        return Err(Error::BadRequest(format!(
            "expected MLS wire format mls_key_package (5), got {wire_format}"
        )));
    }
    Ok(())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Encode a protobuf `ServerEvent` and broadcast it to the given user IDs via
/// SSE. Encoding failures are logged at error level; send failures (e.g. no
/// active receivers) are logged at trace level.
fn broadcast_sse(
    sse_tx: &tokio::sync::broadcast::Sender<SseEvent>,
    event: conclave_proto::ServerEvent,
    target_user_ids: Vec<i64>,
) {
    let mut data = Vec::new();
    if let Err(error) = event.encode(&mut data) {
        tracing::error!(%error, "failed to encode SSE event");
        return;
    }
    if let Err(error) = sse_tx.send(SseEvent {
        data,
        target_user_ids,
    }) {
        tracing::trace!(error = %error, "SSE broadcast dropped (no receivers)");
    }
}

// ── Public Endpoints ──────────────────────────────────────────────

async fn register(State(state): State<Arc<AppState>>, body: Bytes) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::RegisterRequest>(&body)?;

    if request.username.is_empty() || request.password.is_empty() {
        return Err(Error::BadRequest(
            "username and password are required".into(),
        ));
    }

    if request.username.len() > 64 {
        return Err(Error::BadRequest(
            "username must be 64 characters or fewer".into(),
        ));
    }

    // Restrict to safe ASCII characters to prevent homoglyph impersonation,
    // control character injection, and display issues. Must start with an
    // alphanumeric character.
    if !request
        .username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        || !request
            .username
            .starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        return Err(Error::BadRequest(
            "username must start with a letter or digit and contain only ASCII letters, digits, underscores, hyphens, or dots".into(),
        ));
    }

    if request.password.len() < 8 {
        return Err(Error::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }

    if !request.alias.is_empty() {
        validate_alias(&request.alias)?;
    }

    let password_hash = auth::hash_password(&request.password)?;
    let user_id = state.db.create_user(&request.username, &password_hash)?;

    if !request.alias.is_empty() {
        state.db.update_user_alias(user_id, Some(&request.alias))?;
    }

    tracing::info!(user_id, username = %request.username, "user registered");

    Ok(proto_response(
        StatusCode::CREATED,
        &conclave_proto::RegisterResponse { user_id },
    ))
}

async fn login(State(state): State<Arc<AppState>>, body: Bytes) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::LoginRequest>(&body)?;

    let user_record = state.db.get_user_by_username(&request.username)?;

    let user_record = match user_record {
        Some(record) => record,
        None => {
            // Timing equalization: perform dummy password verification to
            // prevent distinguishing "user not found" from "wrong password"
            // via timing. The result is intentionally unused.
            let _ = auth::verify_password("dummy", auth::dummy_hash());
            return Err(Error::Unauthorized("invalid username or password".into()));
        }
    };

    if !auth::verify_password(&request.password, &user_record.password_hash)? {
        return Err(Error::Unauthorized("invalid username or password".into()));
    }

    let token = auth::generate_token();
    let expires_at = unix_now() + state.config.token_ttl_seconds;
    state
        .db
        .create_session(&token, user_record.user_id, expires_at)?;

    tracing::info!(user_id = user_record.user_id, username = %user_record.username, "user logged in");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::LoginResponse {
            token,
            user_id: user_record.user_id,
            username: user_record.username,
        },
    ))
}

async fn logout(State(state): State<Arc<AppState>>, auth: AuthUser) -> Result<impl IntoResponse> {
    state.db.delete_session(&auth.token)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Authenticated Endpoints ───────────────────────────────────────

async fn me(State(state): State<Arc<AppState>>, auth: AuthUser) -> Result<impl IntoResponse> {
    let (user_id, username, alias) = state
        .db
        .get_user_by_id(auth.user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UserInfoResponse {
            user_id,
            username,
            alias: alias.unwrap_or_default(),
        },
    ))
}

async fn update_profile(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::UpdateProfileRequest>(&body)?;

    let alias = if request.alias.is_empty() {
        None
    } else {
        Some(request.alias.as_str())
    };

    state.db.update_user_alias(auth.user_id, alias)?;

    // Broadcast GroupUpdateEvent to all co-members so they refresh member lists.
    let user_groups = state.db.list_user_groups(auth.user_id)?;
    for group_row in &user_groups {
        let members = state.db.get_group_members(group_row.group_id)?;
        let target_ids: Vec<i64> = members
            .iter()
            .map(|(id, _, _)| *id)
            .filter(|id| *id != auth.user_id)
            .collect();

        if target_ids.is_empty() {
            continue;
        }

        broadcast_sse(
            &state.sse_tx,
            conclave_proto::ServerEvent {
                event: Some(conclave_proto::server_event::Event::GroupUpdate(
                    conclave_proto::GroupUpdateEvent {
                        group_id: group_row.group_id,
                        update_type: "member_profile".into(),
                    },
                )),
            },
            target_ids,
        );
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UpdateProfileResponse {},
    ))
}

async fn get_user_by_username(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Path(username): Path<String>,
) -> Result<impl IntoResponse> {
    let user = state
        .db
        .get_user_by_username(&username)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UserInfoResponse {
            user_id: user.user_id,
            username: user.username,
            alias: user.alias.unwrap_or_default(),
        },
    ))
}

async fn upload_key_package(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::UploadKeyPackageRequest>(&body)?;

    if !request.entries.is_empty() {
        // Batch upload path.
        for entry in &request.entries {
            if entry.data.is_empty() {
                return Err(Error::BadRequest("key package data is required".into()));
            }
            if entry.data.len() > 16 * 1024 {
                return Err(Error::BadRequest(
                    "key_package_data must be 16 KiB or smaller".into(),
                ));
            }
            validate_key_package_wire_format(&entry.data)?;
            state
                .db
                .store_key_package(auth.user_id, &entry.data, entry.is_last_resort)?;
        }
    } else if !request.key_package_data.is_empty() {
        // Legacy single-upload path (regular key package).
        if request.key_package_data.len() > 16 * 1024 {
            return Err(Error::BadRequest(
                "key_package_data must be 16 KiB or smaller".into(),
            ));
        }
        validate_key_package_wire_format(&request.key_package_data)?;
        state
            .db
            .store_key_package(auth.user_id, &request.key_package_data, false)?;
    } else {
        return Err(Error::BadRequest("key_package_data is required".into()));
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UploadKeyPackageResponse {},
    ))
}

async fn get_key_package(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Path(user_id): Path<i64>,
) -> Result<impl IntoResponse> {
    let data = state
        .db
        .consume_key_package(user_id)?
        .ok_or_else(|| Error::NotFound("no key package available for this user".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::GetKeyPackageResponse {
            key_package_data: data,
        },
    ))
}

async fn create_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::CreateGroupRequest>(&body)?;

    let alias = if request.alias.is_empty() {
        None
    } else {
        Some(request.alias.as_str())
    };

    let group_name = if request.group_name.is_empty() {
        None
    } else {
        Some(request.group_name.as_str())
    };

    // Collect key packages for all requested members (skip the creator).
    let mut member_key_packages = std::collections::HashMap::new();
    for username in &request.member_usernames {
        let member_id = state
            .db
            .get_user_id_by_username(username)?
            .ok_or_else(|| Error::NotFound(format!("user '{username}' not found")))?;

        if member_id == auth.user_id {
            continue;
        }

        let key_package_data = state.db.consume_key_package(member_id)?.ok_or_else(|| {
            Error::NotFound(format!("no key package available for user '{username}'"))
        })?;

        member_key_packages.insert(username.clone(), key_package_data);
    }

    // Create the group in the database (auto-increment ID).
    let group_id = state.db.create_group(group_name, alias, auth.user_id)?;

    Ok(proto_response(
        StatusCode::CREATED,
        &conclave_proto::CreateGroupResponse {
            group_id,
            member_key_packages,
        },
    ))
}

async fn list_groups(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    let groups = state.db.list_user_groups(auth.user_id)?;

    let mut group_infos = Vec::new();
    for row in groups {
        let members = state.db.get_group_members(row.group_id)?;
        let member_protos = members
            .into_iter()
            .map(|(uid, uname, ualias)| conclave_proto::GroupMember {
                user_id: uid,
                username: uname,
                alias: ualias.unwrap_or_default(),
            })
            .collect();

        group_infos.push(conclave_proto::GroupInfo {
            group_id: row.group_id,
            alias: row.alias.unwrap_or_default(),
            group_name: row.group_name.unwrap_or_default(),
            creator_id: row.creator_id,
            members: member_protos,
            created_at: row.created_at as u64,
            mls_group_id: row.mls_group_id.unwrap_or_default(),
        });
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListGroupsResponse {
            groups: group_infos,
        },
    ))
}

async fn update_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::UpdateGroupRequest>(&body)?;

    // Only the creator can update group settings.
    let creator_id = state
        .db
        .get_group_creator(group_id)?
        .ok_or_else(|| Error::NotFound("group not found".into()))?;
    if creator_id != auth.user_id {
        return Err(Error::Unauthorized(
            "only the group creator can update group settings".into(),
        ));
    }

    if !request.alias.is_empty() {
        state
            .db
            .update_group_alias(group_id, Some(&request.alias))?;
    }
    if !request.group_name.is_empty() {
        state
            .db
            .update_group_name(group_id, Some(&request.group_name))?;
    }

    // Broadcast GroupUpdateEvent so other members refresh the room list.
    let members = state.db.get_group_members(group_id)?;
    let target_ids: Vec<i64> = members
        .iter()
        .map(|(id, _, _)| *id)
        .filter(|id| *id != auth.user_id)
        .collect();

    if !target_ids.is_empty() {
        broadcast_sse(
            &state.sse_tx,
            conclave_proto::ServerEvent {
                event: Some(conclave_proto::server_event::Event::GroupUpdate(
                    conclave_proto::GroupUpdateEvent {
                        group_id,
                        update_type: "group_settings".into(),
                    },
                )),
            },
            target_ids,
        );
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UpdateGroupResponse {},
    ))
}

async fn invite_to_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::InviteToGroupRequest>(&body)?;

    if request.usernames.is_empty() {
        return Err(Error::BadRequest(
            "at least one username is required".into(),
        ));
    }

    // Verify the inviter is a group member.
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    // Collect key packages for the invitees.
    let mut member_key_packages = std::collections::HashMap::new();
    for username in &request.usernames {
        let member_id = state
            .db
            .get_user_id_by_username(username)?
            .ok_or_else(|| Error::NotFound(format!("user '{username}' not found")))?;

        if member_id == auth.user_id {
            continue;
        }

        if state.db.is_group_member(group_id, member_id)? {
            return Err(Error::Conflict(format!(
                "user '{username}' is already a member of this group"
            )));
        }

        let key_package_data = state.db.consume_key_package(member_id)?.ok_or_else(|| {
            Error::NotFound(format!("no key package available for user '{username}'"))
        })?;

        member_key_packages.insert(username.clone(), key_package_data);
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::InviteToGroupResponse {
            member_key_packages,
        },
    ))
}

async fn upload_commit(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::UploadCommitRequest>(&body)?;

    // Verify the sender is a group member.
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    // Get group alias for welcome messages.
    let group_alias = state.db.get_group_alias(group_id)?;

    // Perform all DB operations atomically in a single transaction.
    let commit_result = state.db.process_commit(
        group_id,
        group_alias.as_deref(),
        auth.user_id,
        &request.welcome_messages,
        &request.group_info,
        &request.commit_message,
    )?;

    if !request.mls_group_id.is_empty() {
        state.db.set_mls_group_id(group_id, &request.mls_group_id)?;
    }

    // Send SSE notifications after the transaction has committed.
    for member in &commit_result.new_members {
        broadcast_sse(
            &state.sse_tx,
            conclave_proto::ServerEvent {
                event: Some(conclave_proto::server_event::Event::Welcome(
                    conclave_proto::WelcomeEvent {
                        group_id,
                        group_alias: group_alias.clone().unwrap_or_default(),
                    },
                )),
            },
            vec![member.user_id],
        );
    }

    if !request.commit_message.is_empty() {
        // Notify existing members (excluding sender and newly added members).
        let members = state.db.get_group_members(group_id)?;
        let new_member_ids: std::collections::HashSet<i64> = commit_result
            .new_members
            .iter()
            .map(|m| m.user_id)
            .collect();
        let member_ids: Vec<i64> = members
            .iter()
            .map(|(id, _, _)| *id)
            .filter(|id| *id != auth.user_id && !new_member_ids.contains(id))
            .collect();

        broadcast_sse(
            &state.sse_tx,
            conclave_proto::ServerEvent {
                event: Some(conclave_proto::server_event::Event::GroupUpdate(
                    conclave_proto::GroupUpdateEvent {
                        group_id,
                        update_type: "commit".into(),
                    },
                )),
            },
            member_ids,
        );
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UploadCommitResponse {},
    ))
}

async fn send_message(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::SendMessageRequest>(&body)?;

    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let sequence_num = state
        .db
        .store_message(group_id, auth.user_id, &request.mls_message)?;

    // Notify group members via SSE.
    let members = state.db.get_group_members(group_id)?;
    let member_ids: Vec<i64> = members
        .iter()
        .map(|(id, _, _)| *id)
        .filter(|id| *id != auth.user_id)
        .collect();

    broadcast_sse(
        &state.sse_tx,
        conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::NewMessage(
                conclave_proto::NewMessageEvent {
                    group_id,
                    sequence_num: sequence_num as u64,
                    sender_id: auth.user_id,
                },
            )),
        },
        member_ids,
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::SendMessageResponse {
            sequence_num: sequence_num as u64,
        },
    ))
}

#[derive(serde::Deserialize)]
pub struct GetMessagesQuery {
    #[serde(default)]
    after: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    100
}

async fn get_messages(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let limit = query.limit.min(500);
    let messages = state.db.get_messages(group_id, query.after, limit)?;

    let stored_messages: Vec<conclave_proto::StoredMessage> = messages
        .into_iter()
        .map(|row| conclave_proto::StoredMessage {
            sequence_num: row.sequence_num as u64,
            sender_id: row.sender_id,
            sender_username: row.sender_username,
            sender_alias: row.sender_alias.unwrap_or_default(),
            mls_message: row.mls_message,
            created_at: row.created_at as u64,
        })
        .collect();

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::GetMessagesResponse {
            messages: stored_messages,
        },
    ))
}

async fn list_pending_welcomes(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    let welcomes = state.db.get_pending_welcomes(auth.user_id)?;

    let pending: Vec<conclave_proto::PendingWelcome> = welcomes
        .into_iter()
        .map(|row| conclave_proto::PendingWelcome {
            group_id: row.group_id,
            group_alias: row.group_alias.unwrap_or_default(),
            welcome_message: row.welcome_data,
            welcome_id: row.welcome_id,
        })
        .collect();

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListPendingWelcomesResponse { welcomes: pending },
    ))
}

async fn accept_welcome(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(welcome_id): Path<i64>,
) -> Result<impl IntoResponse> {
    // Verify the welcome belongs to this user by checking pending_welcomes.
    let welcomes = state.db.get_pending_welcomes(auth.user_id)?;
    if !welcomes.iter().any(|row| row.welcome_id == welcome_id) {
        return Err(Error::NotFound("welcome not found".into()));
    }

    state.db.delete_pending_welcome(welcome_id, auth.user_id)?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Member Management ─────────────────────────────────────────────

async fn remove_group_member(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::RemoveMemberRequest>(&body)?;

    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let target_user_id = state
        .db
        .get_user_id_by_username(&request.username)?
        .ok_or_else(|| Error::NotFound(format!("user '{}' not found", request.username)))?;

    if !state.db.is_group_member(group_id, target_user_id)? {
        return Err(Error::BadRequest(format!(
            "user '{}' is not a member of this group",
            request.username
        )));
    }

    // Store the group info from the removal commit.
    if !request.group_info.is_empty() {
        state.db.store_group_info(group_id, &request.group_info)?;
    }

    // Store the commit message for other members to process.
    if !request.commit_message.is_empty() {
        state
            .db
            .store_message(group_id, auth.user_id, &request.commit_message)?;
    }

    // Remove from server DB.
    state.db.remove_group_member(group_id, target_user_id)?;

    // Notify all remaining members via SSE.
    let members = state.db.get_group_members(group_id)?;
    let member_ids: Vec<i64> = members.iter().map(|(id, _, _)| *id).collect();

    // Notify remaining members and the removed user.
    let mut all_targets = member_ids;
    all_targets.push(target_user_id);
    broadcast_sse(
        &state.sse_tx,
        conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::MemberRemoved(
                conclave_proto::MemberRemovedEvent {
                    group_id,
                    removed_username: request.username.clone(),
                },
            )),
        },
        all_targets,
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::RemoveMemberResponse {},
    ))
}

async fn leave_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::LeaveGroupRequest>(&body)?;

    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let (_, username, _) = state
        .db
        .get_user_by_id(auth.user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    // Store the group info from the leave commit (for external rejoin).
    if !request.group_info.is_empty() {
        state.db.store_group_info(group_id, &request.group_info)?;
    }

    // Store the commit message so remaining members can process the MLS
    // removal and advance their epoch.
    if !request.commit_message.is_empty() {
        state
            .db
            .store_message(group_id, auth.user_id, &request.commit_message)?;
    }

    // Remove from server DB.
    state.db.remove_group_member(group_id, auth.user_id)?;

    // Notify remaining members via SSE.
    let members = state.db.get_group_members(group_id)?;
    let member_ids: Vec<i64> = members.iter().map(|(id, _, _)| *id).collect();

    broadcast_sse(
        &state.sse_tx,
        conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::MemberRemoved(
                conclave_proto::MemberRemovedEvent {
                    group_id,
                    removed_username: username,
                },
            )),
        },
        member_ids,
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::LeaveGroupResponse {},
    ))
}

async fn get_group_info(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let group_info_data = state
        .db
        .get_group_info(group_id)?
        .ok_or_else(|| Error::NotFound("no group info available".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::GetGroupInfoResponse {
            group_info: group_info_data,
        },
    ))
}

async fn external_join(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::ExternalJoinRequest>(&body)?;

    // Verify the group exists.
    if !state.db.group_exists(group_id)? {
        return Err(Error::NotFound("group not found".into()));
    }

    // Only existing members (e.g., after an account reset that preserves
    // server-side memberships) may rejoin via external commit.
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    if state.db.get_group_info(group_id)?.is_none() {
        return Err(Error::BadRequest(
            "no group info available for external join".into(),
        ));
    }

    if !request.mls_group_id.is_empty() {
        state.db.set_mls_group_id(group_id, &request.mls_group_id)?;
    }

    // Store the external commit as a message for other members to process.
    if !request.commit_message.is_empty() {
        state
            .db
            .store_message(group_id, auth.user_id, &request.commit_message)?;

        // Notify existing members about the identity reset.
        let members = state.db.get_group_members(group_id)?;
        let member_ids: Vec<i64> = members
            .iter()
            .map(|(id, _, _)| *id)
            .filter(|id| *id != auth.user_id)
            .collect();

        let reset_username = state
            .db
            .get_user_by_id(auth.user_id)?
            .map(|(_, username, _)| username)
            .unwrap_or_else(|| format!("user#{}", auth.user_id));

        broadcast_sse(
            &state.sse_tx,
            conclave_proto::ServerEvent {
                event: Some(conclave_proto::server_event::Event::IdentityReset(
                    conclave_proto::IdentityResetEvent {
                        group_id,
                        username: reset_username,
                    },
                )),
            },
            member_ids,
        );
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ExternalJoinResponse {},
    ))
}

async fn reset_account(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    // Delete all key packages for this user.
    state.db.delete_key_packages(auth.user_id)?;

    tracing::info!(user_id = auth.user_id, "account reset");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ResetAccountResponse {},
    ))
}

// ── SSE ───────────────────────────────────────────────────────────

async fn sse_stream(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, std::convert::Infallible>>> {
    let user_id = auth.user_id;
    tracing::debug!(user_id, "SSE client connected");
    let rx = state.sse_tx.subscribe();

    let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
        Ok(sse_event) if sse_event.target_user_ids.contains(&user_id) => {
            let encoded = hex::encode(&sse_event.data);
            Some(Ok(Event::default().data(encoded)))
        }
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(count)) => {
            tracing::warn!(
                user_id = user_id,
                count = count,
                "SSE client lagged, events dropped"
            );
            Some(Ok(Event::default().event("lagged").data(count.to_string())))
        }
        _ => None,
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
