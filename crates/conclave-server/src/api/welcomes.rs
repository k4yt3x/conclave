use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{Error, Result};
use crate::state::AppState;

use super::proto_response;

pub async fn list_pending_welcomes(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Result<impl IntoResponse> {
    let welcomes = state.db.get_pending_welcomes(auth.user_id)?;

    let pending: Vec<conclave_proto::PendingWelcome> = welcomes
        .into_iter()
        .map(|row| conclave_proto::PendingWelcome {
            group_id: row.group_id.as_bytes().to_vec(),
            group_alias: row.group_alias.unwrap_or_default(),
            welcome_message: row.welcome_data,
            welcome_id: row.welcome_id.as_bytes().to_vec(),
        })
        .collect();

    Ok(proto_response(
        StatusCode::OK,
        &conclave_proto::ListPendingWelcomesResponse { welcomes: pending },
    ))
}

pub async fn accept_welcome(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(welcome_id): Path<Uuid>,
) -> Result<impl IntoResponse> {
    let welcomes = state.db.get_pending_welcomes(auth.user_id)?;
    if !welcomes.iter().any(|row| row.welcome_id == welcome_id) {
        return Err(Error::NotFound("welcome not found".into()));
    }

    state.db.delete_pending_welcome(welcome_id, auth.user_id)?;

    Ok(StatusCode::NO_CONTENT)
}
