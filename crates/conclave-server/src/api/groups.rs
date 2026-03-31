use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;
use crate::validation::{validate_group_name, validate_visibility};

use super::{broadcast_sse, decode_proto, notify_group_members, proto_response};

/// Convert a DB role string to the protobuf `GroupRole` enum value.
fn role_to_proto(role: &str) -> i32 {
    match role {
        "admin" => conclave_proto::GroupRole::Admin.into(),
        _ => conclave_proto::GroupRole::Member.into(),
    }
}

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
        &conclave_proto::CreateGroupResponse {
            group_id: group_id.as_bytes().to_vec(),
        },
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
                user_id: m.user_id.as_bytes().to_vec(),
                username: m.username,
                alias: m.alias.unwrap_or_default(),
                role: role_to_proto(&m.role),
                signing_key_fingerprint: m.signing_key_fingerprint.unwrap_or_default(),
            })
            .collect();

        group_infos.push(conclave_proto::GroupInfo {
            group_id: row.group_id.as_bytes().to_vec(),
            alias: row.alias.unwrap_or_default(),
            group_name: row.group_name,
            members: member_protos,
            mls_group_id: row.mls_group_id.unwrap_or_default(),
            message_expiry_seconds: row.message_expiry_seconds,
            visibility: row.visibility,
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
    Path(group_id): Path<Uuid>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::UpdateGroupRequest>(&body)?;

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::not_admin(
            "only group admins can update group settings",
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
            return Err(Error::BadRequest(
                "group expiry cannot exceed server retention policy".into(),
            ));
        }

        state.db.set_group_expiry(group_id, seconds)?;
    }

    if request.visibility != 0 {
        validate_visibility(request.visibility)?;
        state
            .db
            .set_group_visibility(group_id, request.visibility)?;
    }

    notify_group_members(
        &state,
        group_id,
        None,
        conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
            group_id: group_id.as_bytes().to_vec(),
            update_type: conclave_proto::GroupUpdateType::GroupSettings.into(),
        }),
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UpdateGroupResponse {},
    ))
}

pub async fn delete_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::not_admin("only group admins can delete a group"));
    }

    // Collect all group members for SSE before deletion.
    let members = state.db.get_group_members(group_id)?;
    let target_ids: Vec<Uuid> = members.iter().map(|m| m.user_id).collect();

    state.db.delete_group(group_id)?;

    if !target_ids.is_empty() {
        broadcast_sse(
            &state.sse_tx,
            conclave_proto::ServerEvent {
                event: Some(conclave_proto::server_event::Event::GroupDeleted(
                    conclave_proto::GroupDeletedEvent {
                        group_id: group_id.as_bytes().to_vec(),
                    },
                )),
            },
            target_ids,
        );
    }

    tracing::info!(group_id = %group_id, user_id = %auth.user_id, "group deleted");

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::DeleteGroupResponse {},
    ))
}

pub async fn get_retention_policy(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::not_member("not a member of this group"));
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
    Path(group_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::not_member("not a member of this group"));
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

#[derive(serde::Deserialize)]
pub struct ListPublicGroupsQuery {
    pub pattern: Option<String>,
}

pub async fn list_public_groups(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    axum::extract::Query(query): axum::extract::Query<ListPublicGroupsQuery>,
) -> Result<impl IntoResponse> {
    let rows = state.db.list_public_groups(query.pattern.as_deref())?;

    let groups = rows
        .into_iter()
        .map(|row| conclave_proto::PublicGroupInfo {
            group_id: row.group_id.as_bytes().to_vec(),
            group_name: row.group_name,
            alias: row.alias.unwrap_or_default(),
            member_count: row.member_count,
        })
        .collect();

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListPublicGroupsResponse { groups },
    ))
}

pub async fn join_public_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    if !state.db.group_exists(group_id)? {
        return Err(Error::NotFound("group not found".into()));
    }

    let visibility = state.db.get_group_visibility(group_id)?;
    if visibility != conclave_proto::GroupVisibility::Public as i32 {
        return Err(Error::not_public("this group is not public"));
    }

    if state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Conflict("already a member of this group".into()));
    }

    if state.db.is_banned(group_id, auth.user_id)? {
        return Err(Error::banned("you are banned from this group"));
    }

    // Check for pending invite.
    if state.db.has_pending_invite(group_id, auth.user_id)? {
        return Err(Error::Conflict(
            "you have a pending invite for this group".into(),
        ));
    }

    let group_info_data = state
        .db
        .get_group_info(group_id)?
        .ok_or_else(|| Error::BadRequest("no group info available for this group".into()))?;

    state.db.add_group_member(group_id, auth.user_id)?;
    state.db.add_pending_external_join(group_id, auth.user_id)?;

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::GetGroupInfoResponse {
            group_info: group_info_data,
        },
    ))
}
