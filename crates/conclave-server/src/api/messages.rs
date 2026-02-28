use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;

use super::{decode_proto, notify_group_members, proto_response};

pub async fn upload_commit(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::UploadCommitRequest>(&body)?;

    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    state.db.process_commit(
        group_id,
        auth.user_id,
        &request.group_info,
        &request.commit_message,
    )?;

    if !request.mls_group_id.is_empty() {
        state.db.set_mls_group_id(group_id, &request.mls_group_id)?;
    }

    if !request.commit_message.is_empty() {
        notify_group_members(
            &state,
            group_id,
            Some(auth.user_id),
            conclave_proto::server_event::Event::GroupUpdate(conclave_proto::GroupUpdateEvent {
                group_id,
                update_type: "commit".into(),
            }),
        );
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::UploadCommitResponse {},
    ))
}

pub async fn send_message(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    body: Bytes,
) -> Result<impl IntoResponse> {
    let request = decode_proto::<conclave_proto::SendMessageRequest>(&body)?;

    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let sequence_num = state
        .db
        .store_message(group_id, auth.user_id, &request.mls_message)?;

    // Update sender's fetch watermark for fetch-then-delete groups.
    let group_expiry = state.db.get_group_expiry(group_id)?;
    if group_expiry == 0 {
        state
            .db
            .update_fetch_watermark(group_id, auth.user_id, sequence_num)?;
    }

    notify_group_members(
        &state,
        group_id,
        Some(auth.user_id),
        conclave_proto::server_event::Event::NewMessage(conclave_proto::NewMessageEvent {
            group_id,
            sequence_num: sequence_num as u64,
            sender_id: auth.user_id,
        }),
    );

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

pub async fn get_messages(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(group_id): Path<i64>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<impl IntoResponse> {
    if !state.db.is_group_member(group_id, auth.user_id)? {
        return Err(Error::Unauthorized("not a member of this group".into()));
    }

    let limit = query.limit.min(500);
    let messages = state.db.get_messages(group_id, query.after, limit)?;

    let max_seq = messages.last().map(|m| m.sequence_num);

    let stored_messages: Vec<conclave_proto::StoredMessage> = messages
        .into_iter()
        .map(|row| conclave_proto::StoredMessage {
            sequence_num: row.sequence_num as u64,
            sender_id: row.sender_id,
            mls_message: row.mls_message,
            created_at: row.created_at as u64,
        })
        .collect();

    // Update fetch watermark for fetch-then-delete groups.
    if let Some(seq) = max_seq {
        let group_expiry = state.db.get_group_expiry(group_id)?;
        if group_expiry == 0 {
            state
                .db
                .update_fetch_watermark(group_id, auth.user_id, seq)?;
        }
    }

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::GetMessagesResponse {
            messages: stored_messages,
        },
    ))
}
