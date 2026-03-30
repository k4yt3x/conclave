use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;

use super::{broadcast_sse, decode_proto, notify_group_members, parse_uuid, proto_response};

pub async fn invite_to_group(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<Uuid>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::InviteToGroupRequest>(&body)?;

    if request.user_ids.is_empty() {
        return Err(Error::BadRequest("at least one user_id is required".into()));
    }

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::not_admin("only group admins can invite members"));
    }

    let mut member_key_packages = Vec::new();
    for member_id_bytes in &request.user_ids {
        let member_id = parse_uuid(member_id_bytes, "user_id")?;
        state
            .db
            .get_user_by_id(member_id)?
            .ok_or_else(|| Error::NotFound("user not found".into()))?;

        if member_id == auth.user_id {
            continue;
        }

        if state.db.is_group_member(group_id, member_id)? {
            return Err(Error::Conflict(
                "user is already a member of this group".into(),
            ));
        }

        let key_package_data = state
            .db
            .consume_key_package(member_id)?
            .ok_or_else(|| Error::NotFound("no key package available for this user".into()))?;

        member_key_packages.push(conclave_proto::MemberKeyPackage {
            user_id: member_id.as_bytes().to_vec(),
            key_package_data,
        });
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
    Path(group_id): Path<Uuid>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::RemoveMemberRequest>(&body)?;

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::not_admin("only group admins can remove members"));
    }

    let target_user_id = parse_uuid(&request.user_id, "user_id")?;
    state
        .db
        .get_user_by_id(target_user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    if !state.db.is_group_member(group_id, target_user_id)? {
        return Err(Error::BadRequest(
            "user is not a member of this group".into(),
        ));
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
    let mut all_targets: Vec<Uuid> = members.iter().map(|m| m.user_id).collect();
    all_targets.push(target_user_id);
    broadcast_sse(
        &state.sse_tx,
        conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::MemberRemoved(
                conclave_proto::MemberRemovedEvent {
                    group_id: group_id.as_bytes().to_vec(),
                    removed_user_id: target_user_id.as_bytes().to_vec(),
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
    Path(group_id): Path<Uuid>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::LeaveGroupRequest>(&body)?;

    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::not_member("not a member of this group"));
    }

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
    let member_ids: Vec<Uuid> = members.iter().map(|m| m.user_id).collect();

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

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::LeaveGroupResponse {},
    ))
}

pub async fn promote_member(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<Uuid>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::PromoteMemberRequest>(&body)?;

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::not_admin("only group admins can promote members"));
    }

    let target_user_id = parse_uuid(&request.user_id, "user_id")?;
    state
        .db
        .get_user_by_id(target_user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    if !state.db.is_group_member(group_id, target_user_id)? {
        return Err(Error::BadRequest(
            "user is not a member of this group".into(),
        ));
    }

    if state.db.is_group_admin(group_id, target_user_id)? {
        return Err(Error::Conflict("user is already an admin".into()));
    }

    state.db.promote_member(group_id, target_user_id)?;

    notify_group_members(
        &state,
        group_id,
        None,
        conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
            group_id: group_id.as_bytes().to_vec(),
            update_type: conclave_proto::GroupUpdateType::RoleChange.into(),
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
    Path(group_id): Path<Uuid>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::DemoteMemberRequest>(&body)?;

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::not_admin("only group admins can demote members"));
    }

    let target_user_id = parse_uuid(&request.user_id, "user_id")?;
    state
        .db
        .get_user_by_id(target_user_id)?
        .ok_or_else(|| Error::NotFound("user not found".into()))?;

    if !state.db.is_group_admin(group_id, target_user_id)? {
        return Err(Error::BadRequest("user is not an admin".into()));
    }

    let admin_count = state.db.count_group_admins(group_id)?;
    if admin_count <= 1 {
        return Err(Error::BadRequest("cannot demote the last admin".into()));
    }

    state.db.demote_member(group_id, target_user_id)?;

    notify_group_members(
        &state,
        group_id,
        None,
        conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
            group_id: group_id.as_bytes().to_vec(),
            update_type: conclave_proto::GroupUpdateType::RoleChange.into(),
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
    Path(group_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::not_member("not a member of this group"));
    }

    let admins = state.db.get_group_admins(group_id)?;
    let admin_protos = admins
        .into_iter()
        .map(|admin| conclave_proto::GroupMember {
            user_id: admin.user_id.as_bytes().to_vec(),
            username: admin.username,
            alias: admin.alias.unwrap_or_default(),
            role: conclave_proto::GroupRole::Admin.into(),
            signing_key_fingerprint: admin.signing_key_fingerprint.unwrap_or_default(),
        })
        .collect();

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListAdminsResponse {
            admins: admin_protos,
        },
    ))
}
