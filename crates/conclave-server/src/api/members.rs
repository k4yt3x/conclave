use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;

use super::{broadcast_sse, decode_proto, notify_group_members, proto_response};

pub async fn invite_to_group(
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

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::Unauthorized(
            "only group admins can invite members".into(),
        ));
    }

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

pub async fn remove_group_member(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::RemoveMemberRequest>(&body)?;

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::Unauthorized(
            "only group admins can remove members".into(),
        ));
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

    if !request.group_info.is_empty() {
        state.db.store_group_info(group_id, &request.group_info)?;
    }

    if !request.commit_message.is_empty() {
        state
            .db
            .store_message(group_id, auth.user_id, &request.commit_message)?;
    }

    state.db.remove_group_member(group_id, target_user_id)?;

    // Notify all remaining members and the removed user.
    let members = state.db.get_group_members(group_id)?;
    let mut all_targets: Vec<i64> = members.iter().map(|m| m.user_id).collect();
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

pub async fn leave_group(
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

    if !request.group_info.is_empty() {
        state.db.store_group_info(group_id, &request.group_info)?;
    }

    if !request.commit_message.is_empty() {
        state
            .db
            .store_message(group_id, auth.user_id, &request.commit_message)?;
    }

    state.db.remove_group_member(group_id, auth.user_id)?;

    // Notify remaining members.
    let members = state.db.get_group_members(group_id)?;
    let member_ids: Vec<i64> = members.iter().map(|m| m.user_id).collect();

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

pub async fn promote_member(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::PromoteMemberRequest>(&body)?;

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::Unauthorized(
            "only group admins can promote members".into(),
        ));
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

    if state.db.is_group_admin(group_id, target_user_id)? {
        return Err(Error::Conflict(format!(
            "user '{}' is already an admin",
            request.username
        )));
    }

    state.db.promote_member(group_id, target_user_id)?;

    notify_group_members(
        &state,
        group_id,
        Some(auth.user_id),
        conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
            group_id,
            update_type: "role_change".into(),
        }),
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::PromoteMemberResponse {},
    ))
}

pub async fn demote_member(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::DemoteMemberRequest>(&body)?;

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::Unauthorized(
            "only group admins can demote members".into(),
        ));
    }

    let target_user_id = state
        .db
        .get_user_id_by_username(&request.username)?
        .ok_or_else(|| Error::NotFound(format!("user '{}' not found", request.username)))?;

    if !state.db.is_group_admin(group_id, target_user_id)? {
        return Err(Error::BadRequest(format!(
            "user '{}' is not an admin",
            request.username
        )));
    }

    let admin_count = state.db.count_group_admins(group_id)?;
    if admin_count <= 1 {
        return Err(Error::BadRequest("cannot demote the last admin".into()));
    }

    state.db.demote_member(group_id, target_user_id)?;

    notify_group_members(
        &state,
        group_id,
        Some(auth.user_id),
        conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
            group_id,
            update_type: "role_change".into(),
        }),
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::DemoteMemberResponse {},
    ))
}

pub async fn list_admins(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let admins = state.db.get_group_admins(group_id)?;
    let admin_protos = admins
        .into_iter()
        .map(|(uid, uname, ualias)| conclave_proto::GroupMember {
            user_id: uid,
            username: uname,
            alias: ualias.unwrap_or_default(),
            role: "admin".into(),
        })
        .collect();

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListAdminsResponse {
            admins: admin_protos,
        },
    ))
}
