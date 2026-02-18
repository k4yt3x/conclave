use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use prost::Message;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::auth::{self, AuthUser};
use crate::error::{Error, Result};
use crate::state::AppState;

/// Build the axum router with all API routes.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Public endpoints
        .route("/api/v1/register", post(register))
        .route("/api/v1/login", post(login))
        // Authenticated endpoints
        .route("/api/v1/me", get(me))
        .route("/api/v1/key-packages", post(upload_key_package))
        .route("/api/v1/key-packages/{user_id}", get(get_key_package))
        .route("/api/v1/groups", post(create_group).get(list_groups))
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
    msg.encode(&mut body).unwrap();
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

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

// ── Public Endpoints ──────────────────────────────────────────────

async fn register(State(state): State<Arc<AppState>>, body: Bytes) -> Result<impl IntoResponse> {
    let req = decode_proto::<conclave_proto::RegisterRequest>(&body)?;

    if req.username.is_empty() || req.password.is_empty() {
        return Err(Error::BadRequest(
            "username and password are required".into(),
        ));
    }

    if req.username.len() > 64 {
        return Err(Error::BadRequest(
            "username must be 64 characters or fewer".into(),
        ));
    }

    if req.password.len() < 8 {
        return Err(Error::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }

    let password_hash = auth::hash_password(&req.password)?;
    let user_id = state.db.create_user(&req.username, &password_hash)?;

    Ok(proto_response(
        StatusCode::CREATED,
        &conclave_proto::RegisterResponse {
            user_id: user_id as u64,
        },
    ))
}

async fn login(State(state): State<Arc<AppState>>, body: Bytes) -> Result<impl IntoResponse> {
    let req = decode_proto::<conclave_proto::LoginRequest>(&body)?;

    let user_record = state.db.get_user_by_username(&req.username)?;

    let (user_id, _username, password_hash) = match user_record {
        Some(record) => record,
        None => {
            // Hash a dummy password to equalize timing and prevent username enumeration.
            let _ = auth::hash_password("dummy_timing_equalization");
            return Err(Error::Unauthorized("invalid username or password".into()));
        }
    };

    if !auth::verify_password(&req.password, &password_hash)? {
        return Err(Error::Unauthorized("invalid username or password".into()));
    }

    let token = auth::generate_token();
    let expires_at = unix_now() + state.config.token_ttl_seconds;
    state.db.create_session(&token, user_id, expires_at)?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::LoginResponse {
            token,
            user_id: user_id as u64,
        },
    ))
}

async fn logout(State(state): State<Arc<AppState>>, auth: AuthUser) -> Result<impl IntoResponse> {
    state.db.delete_session(&auth.token)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Authenticated Endpoints ───────────────────────────────────────

async fn me(State(state): State<Arc<AppState>>, auth: AuthUser) -> Result<impl IntoResponse> {
    let (user_id, username) = state
        .db
        .get_user_by_id(auth.user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UserInfoResponse {
            user_id: user_id as u64,
            username,
        },
    ))
}

async fn get_user_by_username(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Path(username): Path<String>,
) -> Result<impl IntoResponse> {
    let (user_id, username, _) = state
        .db
        .get_user_by_username(&username)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UserInfoResponse {
            user_id: user_id as u64,
            username,
        },
    ))
}

async fn upload_key_package(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let req = decode_proto::<conclave_proto::UploadKeyPackageRequest>(&body)?;

    if !req.entries.is_empty() {
        // Batch upload path.
        for entry in &req.entries {
            if entry.data.is_empty() {
                return Err(Error::BadRequest("key package data is required".into()));
            }
            if entry.data.len() > 16 * 1024 {
                return Err(Error::BadRequest(
                    "key_package_data must be 16 KiB or smaller".into(),
                ));
            }
            state
                .db
                .store_key_package(auth.user_id, &entry.data, entry.is_last_resort)?;
        }
    } else if !req.key_package_data.is_empty() {
        // Legacy single-upload path (regular key package).
        if req.key_package_data.len() > 16 * 1024 {
            return Err(Error::BadRequest(
                "key_package_data must be 16 KiB or smaller".into(),
            ));
        }
        state
            .db
            .store_key_package(auth.user_id, &req.key_package_data, false)?;
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
    let req = decode_proto::<conclave_proto::CreateGroupRequest>(&body)?;

    if req.name.is_empty() {
        return Err(Error::BadRequest("group name is required".into()));
    }

    if req.name.len() > 128 {
        return Err(Error::BadRequest(
            "group name must be 128 characters or fewer".into(),
        ));
    }

    let group_id = uuid::Uuid::new_v4().to_string();

    // Collect key packages for all requested members (skip the creator).
    let mut member_key_packages = std::collections::HashMap::new();
    for username in &req.member_usernames {
        let member_id = state
            .db
            .get_user_id_by_username(username)?
            .ok_or_else(|| Error::NotFound(format!("user '{username}' not found")))?;

        if member_id == auth.user_id {
            continue;
        }

        let kp_data = state.db.consume_key_package(member_id)?.ok_or_else(|| {
            Error::NotFound(format!("no key package available for user '{username}'"))
        })?;

        member_key_packages.insert(username.clone(), kp_data);
    }

    // Create the group in the database.
    state.db.create_group(&group_id, &req.name, auth.user_id)?;

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
    for (group_id, name, creator_id, created_at) in groups {
        let members = state.db.get_group_members(&group_id)?;
        let member_protos = members
            .into_iter()
            .map(|(uid, uname)| conclave_proto::GroupMember {
                user_id: uid as u64,
                username: uname,
            })
            .collect();

        group_infos.push(conclave_proto::GroupInfo {
            group_id,
            name,
            creator_id: creator_id as u64,
            members: member_protos,
            created_at: created_at as u64,
        });
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListGroupsResponse {
            groups: group_infos,
        },
    ))
}

async fn invite_to_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let req = decode_proto::<conclave_proto::InviteToGroupRequest>(&body)?;

    if req.usernames.is_empty() {
        return Err(Error::BadRequest(
            "at least one username is required".into(),
        ));
    }

    // Verify the inviter is a group member.
    if !state.db.is_group_member(&group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    // Collect key packages for the invitees.
    let mut member_key_packages = std::collections::HashMap::new();
    for username in &req.usernames {
        let member_id = state
            .db
            .get_user_id_by_username(username)?
            .ok_or_else(|| Error::NotFound(format!("user '{username}' not found")))?;

        if member_id == auth.user_id {
            continue;
        }

        if state.db.is_group_member(&group_id, member_id)? {
            return Err(Error::Conflict(format!(
                "user '{username}' is already a member of this group"
            )));
        }

        let kp_data = state.db.consume_key_package(member_id)?.ok_or_else(|| {
            Error::NotFound(format!("no key package available for user '{username}'"))
        })?;

        member_key_packages.insert(username.clone(), kp_data);
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
    Path(group_id): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let req = decode_proto::<conclave_proto::UploadCommitRequest>(&body)?;

    // Verify the sender is a group member.
    if !state.db.is_group_member(&group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    // Get group name for welcome messages.
    let groups = state.db.list_user_groups(auth.user_id)?;
    let group_name = groups
        .iter()
        .find(|(gid, _, _, _)| gid == &group_id)
        .map(|(_, name, _, _)| name.clone())
        .unwrap_or_default();

    // Store welcome messages for each recipient.
    for (username, welcome_data) in &req.welcome_messages {
        let user_id = state
            .db
            .get_user_id_by_username(username)?
            .ok_or_else(|| Error::NotFound(format!("user '{username}' not found")))?;

        // Add the user as a group member.
        state.db.add_group_member(&group_id, user_id)?;

        // Store the welcome for them to pick up.
        state
            .db
            .store_pending_welcome(&group_id, &group_name, user_id, welcome_data)?;

        // Notify via SSE.
        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::Welcome(
                conclave_proto::WelcomeEvent {
                    group_id: group_id.clone(),
                    group_name: group_name.clone(),
                },
            )),
        };
        let mut event_bytes = Vec::new();
        event.encode(&mut event_bytes).unwrap();
        let _ = state.sse_tx.send(crate::state::SseEvent {
            data: event_bytes,
            target_user_ids: vec![user_id],
        });
    }

    // Store the latest group info for external commits.
    if !req.group_info.is_empty() {
        state.db.store_group_info(&group_id, &req.group_info)?;
    }

    // Store the commit as a message so other existing members can process it.
    if !req.commit_message.is_empty() {
        state
            .db
            .store_message(&group_id, auth.user_id, &req.commit_message)?;

        // Notify existing members.
        let members = state.db.get_group_members(&group_id)?;
        let member_ids: Vec<i64> = members
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| *id != auth.user_id)
            .collect();

        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::GroupUpdate(
                conclave_proto::GroupUpdateEvent {
                    group_id: group_id.clone(),
                    update_type: "commit".into(),
                },
            )),
        };
        let mut event_bytes = Vec::new();
        event.encode(&mut event_bytes).unwrap();
        let _ = state.sse_tx.send(crate::state::SseEvent {
            data: event_bytes,
            target_user_ids: member_ids,
        });
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UploadCommitResponse {},
    ))
}

async fn send_message(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let req = decode_proto::<conclave_proto::SendMessageRequest>(&body)?;

    if !state.db.is_group_member(&group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let sequence_num = state
        .db
        .store_message(&group_id, auth.user_id, &req.mls_message)?;

    // Notify group members via SSE.
    let members = state.db.get_group_members(&group_id)?;
    let member_ids: Vec<i64> = members
        .iter()
        .map(|(id, _)| *id)
        .filter(|id| *id != auth.user_id)
        .collect();

    let event = conclave_proto::ServerEvent {
        event: Some(conclave_proto::server_event::Event::NewMessage(
            conclave_proto::NewMessageEvent {
                group_id: group_id.clone(),
                sequence_num: sequence_num as u64,
                sender_id: auth.user_id as u64,
            },
        )),
    };
    let mut event_bytes = Vec::new();
    event.encode(&mut event_bytes).unwrap();
    let _ = state.sse_tx.send(crate::state::SseEvent {
        data: event_bytes,
        target_user_ids: member_ids,
    });

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
    Path(group_id): Path<String>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(&group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let limit = query.limit.min(500);
    let messages = state.db.get_messages(&group_id, query.after, limit)?;

    let stored_messages: Vec<conclave_proto::StoredMessage> = messages
        .into_iter()
        .map(|(seq, sender_id, sender_username, mls_msg, created_at)| {
            conclave_proto::StoredMessage {
                sequence_num: seq as u64,
                sender_id: sender_id as u64,
                sender_username,
                mls_message: mls_msg,
                created_at: created_at as u64,
            }
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
        .map(
            |(id, group_id, group_name, welcome_data)| conclave_proto::PendingWelcome {
                group_id,
                group_name,
                welcome_message: welcome_data,
                welcome_id: id,
            },
        )
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
    if !welcomes.iter().any(|(id, _, _, _)| *id == welcome_id) {
        return Err(Error::NotFound("welcome not found".into()));
    }

    state.db.delete_pending_welcome(welcome_id, auth.user_id)?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Member Management ─────────────────────────────────────────────

async fn remove_group_member(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let req = decode_proto::<conclave_proto::RemoveMemberRequest>(&body)?;

    if !state.db.is_group_member(&group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let target_user_id = state
        .db
        .get_user_id_by_username(&req.username)?
        .ok_or_else(|| Error::NotFound(format!("user '{}' not found", req.username)))?;

    if !state.db.is_group_member(&group_id, target_user_id)? {
        return Err(Error::BadRequest(format!(
            "user '{}' is not a member of this group",
            req.username
        )));
    }

    // Store the group info from the removal commit.
    if !req.group_info.is_empty() {
        state.db.store_group_info(&group_id, &req.group_info)?;
    }

    // Store the commit message for other members to process.
    if !req.commit_message.is_empty() {
        state
            .db
            .store_message(&group_id, auth.user_id, &req.commit_message)?;
    }

    // Remove from server DB.
    state.db.remove_group_member(&group_id, target_user_id)?;

    // Notify all remaining members via SSE.
    let members = state.db.get_group_members(&group_id)?;
    let member_ids: Vec<i64> = members.iter().map(|(id, _)| *id).collect();

    let event = conclave_proto::ServerEvent {
        event: Some(conclave_proto::server_event::Event::MemberRemoved(
            conclave_proto::MemberRemovedEvent {
                group_id: group_id.clone(),
                removed_username: req.username.clone(),
            },
        )),
    };
    let mut event_bytes = Vec::new();
    event.encode(&mut event_bytes).unwrap();
    let _ = state.sse_tx.send(crate::state::SseEvent {
        data: event_bytes.clone(),
        target_user_ids: member_ids,
    });

    // Also notify the removed user.
    let _ = state.sse_tx.send(crate::state::SseEvent {
        data: event_bytes,
        target_user_ids: vec![target_user_id],
    });

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::RemoveMemberResponse {},
    ))
}

async fn leave_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<String>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(&group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let (_, username) = state
        .db
        .get_user_by_id(auth.user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    // Remove from server DB.
    state.db.remove_group_member(&group_id, auth.user_id)?;

    // Notify remaining members via SSE.
    let members = state.db.get_group_members(&group_id)?;
    let member_ids: Vec<i64> = members.iter().map(|(id, _)| *id).collect();

    let event = conclave_proto::ServerEvent {
        event: Some(conclave_proto::server_event::Event::MemberRemoved(
            conclave_proto::MemberRemovedEvent {
                group_id: group_id.clone(),
                removed_username: username,
            },
        )),
    };
    let mut event_bytes = Vec::new();
    event.encode(&mut event_bytes).unwrap();
    let _ = state.sse_tx.send(crate::state::SseEvent {
        data: event_bytes,
        target_user_ids: member_ids,
    });

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::LeaveGroupResponse {},
    ))
}

async fn get_group_info(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<String>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(&group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let group_info_data = state
        .db
        .get_group_info(&group_id)?
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
    Path(group_id): Path<String>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let req = decode_proto::<conclave_proto::ExternalJoinRequest>(&body)?;

    // Add user as group member (re-add after reset).
    state.db.add_group_member(&group_id, auth.user_id)?;

    // Store the external commit as a message for other members to process.
    if !req.commit_message.is_empty() {
        state
            .db
            .store_message(&group_id, auth.user_id, &req.commit_message)?;

        // Notify existing members.
        let members = state.db.get_group_members(&group_id)?;
        let member_ids: Vec<i64> = members
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| *id != auth.user_id)
            .collect();

        let event = conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::GroupUpdate(
                conclave_proto::GroupUpdateEvent {
                    group_id: group_id.clone(),
                    update_type: "commit".into(),
                },
            )),
        };
        let mut event_bytes = Vec::new();
        event.encode(&mut event_bytes).unwrap();
        let _ = state.sse_tx.send(crate::state::SseEvent {
            data: event_bytes,
            target_user_ids: member_ids,
        });
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
    let rx = state.sse_tx.subscribe();

    let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
        Ok(sse_event) if sse_event.target_user_ids.contains(&user_id) => {
            let encoded = hex::encode(&sse_event.data);
            Some(Ok(Event::default().data(encoded)))
        }
        _ => None,
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
