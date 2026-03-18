use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;

use super::{broadcast_sse, decode_proto, proto_response};

pub async fn external_join(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<Uuid>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::ExternalJoinRequest>(&body)?;

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

    if !request.commit_message.is_empty() {
        state
            .db
            .store_message(group_id, auth.user_id, &request.commit_message)?;

        // Notify existing members about the identity reset.
        let members = state.db.get_group_members(group_id)?;
        let member_ids: Vec<Uuid> = members
            .iter()
            .map(|m| m.user_id)
            .filter(|id| *id != auth.user_id)
            .collect();

        broadcast_sse(
            &state.sse_tx,
            conclave_proto::ServerEvent {
                event: Some(conclave_proto::server_event::Event::IdentityReset(
                    conclave_proto::IdentityResetEvent {
                        group_id: group_id.as_bytes().to_vec(),
                        user_id: auth.user_id.as_bytes().to_vec(),
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
