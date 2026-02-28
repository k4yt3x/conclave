use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::validation::validate_group_name;

use super::{decode_proto, notify_group_members, proto_response};

pub async fn create_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::CreateGroupRequest>(&body)?;

    if request.group_name.is_empty() {
        return Err(Error::BadRequest("group_name is required".into()));
    }

    let alias = if request.alias.is_empty() {
        None
    } else {
        Some(request.alias.as_str())
    };

    let group_id = state
        .db
        .create_group(&request.group_name, alias, auth.user_id)?;

    Ok(proto_response(
        StatusCode::CREATED,
        &conclave_proto::CreateGroupResponse { group_id },
    ))
}

pub async fn list_groups(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    let groups = state.db.list_user_groups(auth.user_id)?;

    let mut group_infos = Vec::new();
    for row in groups {
        let members = state.db.get_group_members(row.group_id)?;
        let member_protos = members
            .into_iter()
            .map(|m| conclave_proto::GroupMember {
                user_id: m.user_id,
                username: m.username,
                alias: m.alias.unwrap_or_default(),
                role: m.role,
            })
            .collect();

        group_infos.push(conclave_proto::GroupInfo {
            group_id: row.group_id,
            alias: row.alias.unwrap_or_default(),
            group_name: row.group_name,
            members: member_protos,
            created_at: row.created_at as u64,
            mls_group_id: row.mls_group_id.unwrap_or_default(),
            message_expiry_seconds: row.message_expiry_seconds,
        });
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListGroupsResponse {
            groups: group_infos,
        },
    ))
}

pub async fn update_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::UpdateGroupRequest>(&body)?;

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::Unauthorized(
            "only group admins can update group settings".into(),
        ));
    }

    if !request.alias.is_empty() {
        state
            .db
            .update_group_alias(group_id, Some(&request.alias))?;
    }
    if !request.group_name.is_empty() {
        validate_group_name(&request.group_name)?;
        state
            .db
            .update_group_name(group_id, Some(&request.group_name))?;
    }
    if request.update_message_expiry {
        let seconds = request.message_expiry_seconds;
        if seconds < -1 {
            return Err(Error::BadRequest(
                "message_expiry_seconds must be -1, 0, or positive".into(),
            ));
        }

        let server_retention = state.config.message_retention_seconds();
        if server_retention > 0 && seconds > 0 && seconds > server_retention {
            return Err(Error::BadRequest(format!(
                "group expiry ({seconds}s) cannot exceed server retention ({server_retention}s)"
            )));
        }

        state.db.set_group_expiry(group_id, seconds)?;
    }

    notify_group_members(
        &state,
        group_id,
        None,
        conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
            group_id,
            update_type: "group_settings".into(),
        }),
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UpdateGroupResponse {},
    ))
}

pub async fn get_retention_policy(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let group_expiry = state.db.get_group_expiry(group_id)?;
    let server_retention = state.config.message_retention_seconds();

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::GetRetentionPolicyResponse {
            server_retention_seconds: server_retention,
            group_expiry_seconds: group_expiry,
        },
    ))
}

pub async fn get_group_info(
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
