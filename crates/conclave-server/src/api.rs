mod auth;
mod external;
mod groups;
mod invites;
mod key_packages;
mod members;
mod messages;
mod sse;
mod welcomes;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use prost::Message;

use crate::error::{Error, Result};
use crate::state::{AppState, SseEvent};

/// Build the axum router with all API routes.
pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        // Public endpoints
        .route("/api/v1/register", post(auth::register))
        .route("/api/v1/login", post(auth::login))
        // Authenticated endpoints
        .route("/api/v1/me", get(auth::me).patch(auth::update_profile))
        .route("/api/v1/key-packages", post(key_packages::upload_key_package))
        .route(
            "/api/v1/key-packages/{user_id}",
            get(key_packages::get_key_package),
        )
        .route(
            "/api/v1/groups",
            post(groups::create_group).get(groups::list_groups),
        )
        .route("/api/v1/groups/{group_id}", patch(groups::update_group))
        .route(
            "/api/v1/groups/{group_id}/invite",
            post(members::invite_to_group),
        )
        .route(
            "/api/v1/groups/{group_id}/commit",
            post(messages::upload_commit),
        )
        .route(
            "/api/v1/groups/{group_id}/messages",
            post(messages::send_message).get(messages::get_messages),
        )
        .route("/api/v1/welcomes", get(welcomes::list_pending_welcomes))
        .route(
            "/api/v1/welcomes/{welcome_id}/accept",
            post(welcomes::accept_welcome),
        )
        .route(
            "/api/v1/groups/{group_id}/escrow-invite",
            post(invites::escrow_invite),
        )
        .route("/api/v1/invites", get(invites::list_pending_invites))
        .route(
            "/api/v1/invites/{invite_id}/accept",
            post(invites::accept_invite),
        )
        .route(
            "/api/v1/invites/{invite_id}/decline",
            post(invites::decline_invite),
        )
        .route(
            "/api/v1/groups/{group_id}/promote",
            post(members::promote_member),
        )
        .route(
            "/api/v1/groups/{group_id}/demote",
            post(members::demote_member),
        )
        .route(
            "/api/v1/groups/{group_id}/admins",
            get(members::list_admins),
        )
        .route(
            "/api/v1/groups/{group_id}/remove",
            post(members::remove_group_member),
        )
        .route(
            "/api/v1/groups/{group_id}/leave",
            post(members::leave_group),
        )
        .route(
            "/api/v1/groups/{group_id}/group-info",
            get(groups::get_group_info),
        )
        .route(
            "/api/v1/groups/{group_id}/external-join",
            post(external::external_join),
        )
        .route("/api/v1/reset-account", post(auth::reset_account))
        .route("/api/v1/logout", post(auth::logout))
        .route("/api/v1/events", get(sse::sse_stream))
        .route(
            "/api/v1/users/{username}",
            get(auth::get_user_by_username),
        )
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

/// Fetch group members, optionally exclude one user, and broadcast an SSE event
/// to all remaining members. Does nothing if the resulting target list is empty.
fn notify_group_members(
    state: &AppState,
    group_id: i64,
    exclude_user_id: Option<i64>,
    event: conclave_proto::server_event::Event,
) {
    let members = match state.db.get_group_members(group_id) {
        Ok(members) => members,
        Err(error) => {
            tracing::warn!(%error, group_id = group_id, "failed to fetch group members for SSE notification");
            return;
        }
    };

    let target_ids: Vec<i64> = members
        .iter()
        .map(|m| m.user_id)
        .filter(|id| exclude_user_id.is_none_or(|exclude| *id != exclude))
        .collect();

    if target_ids.is_empty() {
        return;
    }

    broadcast_sse(
        &state.sse_tx,
        conclave_proto::ServerEvent { event: Some(event) },
        target_ids,
    );
}
