use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use subtle::ConstantTimeEq;

use crate::auth::{self, AuthUser};
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::validation::{validate_alias, validate_password, validate_username};

use super::{broadcast_sse, decode_proto, notify_group_members, proto_response, unix_now};

pub async fn register(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::RegisterRequest>(&body)?;

    if !state.config.registration_enabled {
        match &state.config.registration_token {
            None => {
                return Err(Error::Forbidden("registration is disabled".into()));
            }
            Some(configured_token) => {
                if !bool::from(
                    request
                        .registration_token
                        .as_bytes()
                        .ct_eq(configured_token.as_bytes()),
                ) {
                    return Err(Error::Forbidden("invalid registration token".into()));
                }
            }
        }
    }

    if request.username.is_empty() || request.password.is_empty() {
        return Err(Error::BadRequest(
            "username and password are required".into(),
        ));
    }

    validate_username(&request.username)?;
    validate_password(&request.password)?;

    if !request.alias.is_empty() {
        validate_alias(&request.alias)?;
    }

    let password_hash = auth::hash_password(&request.password)?;
    let user_id = state.db.create_user(&request.username, &password_hash)?;

    if !request.alias.is_empty() {
        state.db.update_user_alias(user_id, Some(&request.alias))?;
    }

    tracing::info!(user_id = %user_id, username = %request.username, "user registered");

    Ok(proto_response(
        StatusCode::CREATED,
        &conclave_proto::RegisterResponse {
            user_id: user_id.as_bytes().to_vec(),
        },
    ))
}

pub async fn login(State(state): State<Arc<AppState>>, body: Bytes) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::LoginRequest>(&body)?;

    let user_record = state.db.get_user_by_username(&request.username)?;

    let user_record = match user_record {
        Some(record) => record,
        None => {
            // Timing equalization: perform dummy password verification to
            // prevent distinguishing "user not found" from "wrong password"
            // via timing. The result is intentionally unused.
            if let Err(error) = auth::verify_password("dummy", auth::dummy_hash()) {
                tracing::warn!(%error, "timing equalization hash verification failed");
            }
            return Err(Error::token_expired("invalid username or password"));
        }
    };

    if !auth::verify_password(&request.password, &user_record.password_hash)? {
        return Err(Error::token_expired("invalid username or password"));
    }

    let token = auth::generate_token();
    let expires_at = unix_now()? + state.config.token_ttl_seconds;
    state
        .db
        .create_session(&token, user_record.user_id, expires_at)?;

    tracing::info!(user_id = %user_record.user_id, username = %user_record.username, "user logged in");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::LoginResponse {
            token,
            user_id: user_record.user_id.as_bytes().to_vec(),
            username: user_record.username,
        },
    ))
}

pub async fn logout(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    state.db.delete_session(&auth.token)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(State(state): State<Arc<AppState>>, auth: AuthUser) -> Result<impl IntoResponse> {
    let user_info = state
        .db
        .get_user_by_id(auth.user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UserInfoResponse {
            user_id: user_info.user_id.as_bytes().to_vec(),
            username: user_info.username,
            alias: user_info.alias.unwrap_or_default(),
            signing_key_fingerprint: user_info.signing_key_fingerprint.unwrap_or_default(),
        },
    ))
}

pub async fn update_profile(
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
        notify_group_members(
            &state,
            group_row.group_id,
            None,
            conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
                group_id: group_row.group_id.as_bytes().to_vec(),
                update_type: "member_profile".into(),
            }),
        );
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UpdateProfileResponse {},
    ))
}

pub async fn get_user_by_username(
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
            user_id: user.user_id.as_bytes().to_vec(),
            username: user.username,
            alias: user.alias.unwrap_or_default(),
            signing_key_fingerprint: user.signing_key_fingerprint.unwrap_or_default(),
        },
    ))
}

pub async fn get_user_by_id(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Path(user_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    let user_info = state
        .db
        .get_user_by_id(user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UserInfoResponse {
            user_id: user_info.user_id.as_bytes().to_vec(),
            username: user_info.username,
            alias: user_info.alias.unwrap_or_default(),
            signing_key_fingerprint: user_info.signing_key_fingerprint.unwrap_or_default(),
        },
    ))
}

pub async fn change_password(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::ChangePasswordRequest>(&body)?;

    let password_hash = state
        .db
        .get_password_hash(auth.user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    if !auth::verify_password(&request.current_password, &password_hash)? {
        return Err(Error::token_expired("invalid password"));
    }

    validate_password(&request.new_password)?;

    let new_hash = auth::hash_password(&request.new_password)?;
    state.db.update_user_password(auth.user_id, &new_hash)?;
    state.db.delete_user_sessions(auth.user_id)?;

    tracing::info!(user_id = %auth.user_id, "password changed");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ChangePasswordResponse {},
    ))
}

pub async fn delete_account(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::DeleteAccountRequest>(&body)?;

    let password_hash = state
        .db
        .get_password_hash(auth.user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    if !auth::verify_password(&request.password, &password_hash)? {
        return Err(Error::token_expired("invalid password"));
    }

    // Collect group memberships and their members for SSE before deletion.
    let user_groups = state.db.list_user_groups(auth.user_id)?;
    let group_members: Vec<(Uuid, Vec<Uuid>)> = user_groups
        .iter()
        .filter_map(|group| {
            state
                .db
                .get_group_members(group.group_id)
                .ok()
                .map(|members| {
                    let ids = members
                        .iter()
                        .filter(|m| m.user_id != auth.user_id)
                        .map(|m| m.user_id)
                        .collect();
                    (group.group_id, ids)
                })
        })
        .collect();

    state.db.delete_user(auth.user_id)?;

    // Broadcast MemberRemovedEvent per group to remaining members.
    for (group_id, member_ids) in group_members {
        if member_ids.is_empty() {
            continue;
        }
        broadcast_sse(
            &state.sse_tx,
            conclave_proto::ServerEvent {
                event: Some(conclave_proto::server_event::Event::MemberRemoved(
                    conclave_proto::MemberRemovedEvent {
                        group_id: group_id.as_bytes().to_vec(),
                        removed_user_id: auth.user_id.as_bytes().to_vec(),
                    },
                )),
            },
            member_ids,
        );
    }

    tracing::info!(user_id = %auth.user_id, "account deleted");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::DeleteAccountResponse {},
    ))
}

pub async fn reset_account(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    state.db.delete_key_packages(auth.user_id)?;

    tracing::info!(user_id = %auth.user_id, "account reset");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ResetAccountResponse {},
    ))
}
