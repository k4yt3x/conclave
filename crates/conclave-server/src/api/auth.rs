use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::auth::{self, AuthUser};
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::validation::{validate_alias, validate_password, validate_username};

use super::{decode_proto, notify_group_members, proto_response, unix_now};

pub async fn register(
    State(state): State<Arc<AppState>>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::RegisterRequest>(&body)?;

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

    tracing::info!(user_id, username = %request.username, "user registered");

    Ok(proto_response(
        StatusCode::CREATED,
        &conclave_proto::RegisterResponse { user_id },
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

pub async fn logout(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    state.db.delete_session(&auth.token)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(State(state): State<Arc<AppState>>, auth: AuthUser) -> Result<impl IntoResponse> {
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
            Some(auth.user_id),
            conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
                group_id: group_row.group_id,
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
            user_id: user.user_id,
            username: user.username,
            alias: user.alias.unwrap_or_default(),
        },
    ))
}

pub async fn get_user_by_id(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    Path(user_id): Path<i64>,
) -> Result<impl IntoResponse> {
    let (uid, username, alias) = state
        .db
        .get_user_by_id(user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UserInfoResponse {
            user_id: uid,
            username,
            alias: alias.unwrap_or_default(),
        },
    ))
}

pub async fn change_password(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::ChangePasswordRequest>(&body)?;

    validate_password(&request.new_password)?;

    let password_hash = state
        .db
        .get_password_hash(auth.user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    if !auth::verify_password(&request.current_password, &password_hash)? {
        return Err(Error::Unauthorized("incorrect current password".into()));
    }

    let new_hash = auth::hash_password(&request.new_password)?;
    state.db.update_user_password(auth.user_id, &new_hash)?;

    tracing::info!(user_id = auth.user_id, "password changed");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ChangePasswordResponse {},
    ))
}

pub async fn reset_account(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    state.db.delete_key_packages(auth.user_id)?;

    tracing::info!(user_id = auth.user_id, "account reset");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ResetAccountResponse {},
    ))
}
