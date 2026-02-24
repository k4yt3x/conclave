use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;

use super::{broadcast_sse, decode_proto, notify_group_members, proto_response};

pub async fn escrow_invite(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::EscrowInviteRequest>(&body)?;

    if request.invitee_username.is_empty() {
        return Err(Error::BadRequest("invitee username is required".into()));
    }

    if request.commit_message.is_empty()
        || request.welcome_message.is_empty()
        || request.group_info.is_empty()
    {
        return Err(Error::BadRequest(
            "commit_message, welcome_message, and group_info are required".into(),
        ));
    }

    if !state.db.is_group_admin(group_id, auth.user_id)? {
        return Err(Error::Unauthorized(
            "only group admins can invite members".into(),
        ));
    }

    let invitee_id = state
        .db
        .get_user_id_by_username(&request.invitee_username)?
        .ok_or_else(|| Error::NotFound(format!("user '{}' not found", request.invitee_username)))?;

    if state.db.is_group_member(group_id, invitee_id)? {
        return Err(Error::Conflict(format!(
            "user '{}' is already a member of this group",
            request.invitee_username
        )));
    }

    let invite_id = state.db.create_pending_invite(
        group_id,
        auth.user_id,
        invitee_id,
        &request.commit_message,
        &request.welcome_message,
        &request.group_info,
    )?;

    let group_alias = state.db.get_group_alias(group_id)?;
    let group_name = state.db.get_group_name(group_id)?.unwrap_or_default();
    let inviter_username = state
        .db
        .get_user_by_id(auth.user_id)?
        .map(|(_, username, _)| username)
        .unwrap_or_default();

    broadcast_sse(
        &state.sse_tx,
        conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::InviteReceived(
                conclave_proto::InviteReceivedEvent {
                    invite_id,
                    group_id,
                    group_name,
                    group_alias: group_alias.unwrap_or_default(),
                    inviter_username,
                },
            )),
        },
        vec![invitee_id],
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::EscrowInviteResponse {},
    ))
}

pub async fn list_pending_invites(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    let rows = state.db.list_pending_invites_for_user(auth.user_id)?;

    let mut invites = Vec::new();
    for row in rows {
        let inviter_username = state
            .db
            .get_user_by_id(row.inviter_id)?
            .map(|(_, username, _)| username)
            .unwrap_or_default();
        let group_name = state.db.get_group_name(row.group_id)?.unwrap_or_default();
        let group_alias = state.db.get_group_alias(row.group_id)?.unwrap_or_default();

        invites.push(conclave_proto::PendingInvite {
            invite_id: row.invite_id,
            group_id: row.group_id,
            group_name,
            group_alias,
            inviter_username,
            created_at: row.created_at as u64,
        });
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListPendingInvitesResponse { invites },
    ))
}

pub async fn accept_invite(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(invite_id): Path<i64>,
) -> Result<impl IntoResponse> {
    let invite = state
        .db
        .get_pending_invite(invite_id)?
        .ok_or_else(|| Error::NotFound("invite not found".into()))?;

    if invite.invitee_id != auth.user_id {
        return Err(Error::Unauthorized(
            "this invite does not belong to you".into(),
        ));
    }

    let result = state.db.accept_pending_invite(invite_id)?;

    // Send WelcomeEvent to the invitee.
    broadcast_sse(
        &state.sse_tx,
        conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::Welcome(
                conclave_proto::WelcomeEvent {
                    group_id: result.group_id,
                    group_alias: result.group_alias.clone().unwrap_or_default(),
                },
            )),
        },
        vec![auth.user_id],
    );

    // Notify existing members with GroupUpdateEvent.
    notify_group_members(
        &state,
        result.group_id,
        Some(auth.user_id),
        conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
            group_id: result.group_id,
            update_type: "commit".into(),
        }),
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::AcceptInviteResponse {},
    ))
}

pub async fn decline_invite(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(invite_id): Path<i64>,
) -> Result<impl IntoResponse> {
    let invite = state
        .db
        .get_pending_invite(invite_id)?
        .ok_or_else(|| Error::NotFound("invite not found".into()))?;

    if invite.invitee_id != auth.user_id {
        return Err(Error::Unauthorized(
            "this invite does not belong to you".into(),
        ));
    }

    let declined_username = state
        .db
        .get_user_by_id(auth.user_id)?
        .map(|(_, username, _)| username)
        .unwrap_or_default();

    state.db.delete_pending_invite(invite_id)?;

    // Notify the inviter so they can clean up the phantom MLS leaf.
    broadcast_sse(
        &state.sse_tx,
        conclave_proto::ServerEvent {
            event: Some(conclave_proto::server_event::Event::InviteDeclined(
                conclave_proto::InviteDeclinedEvent {
                    group_id: invite.group_id,
                    declined_username,
                },
            )),
        },
        vec![invite.inviter_id],
    );

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::DeclineInviteResponse {},
    ))
}
