use std::sync::Arc;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

use crate::auth::AuthUser;
use crate::state::AppState;

pub async fn sse_stream(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
) -> Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, std::convert::Infallible>>> {
    let user_id = auth.user_id;
    tracing::debug!(user_id = %user_id, "SSE client connected");
    let rx = state.sse_tx.subscribe();

    let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
        Ok(sse_event) if sse_event.target_user_ids.contains(&user_id) => {
            let encoded = hex::encode(&sse_event.data);
            Some(Ok(Event::default().data(encoded)))
        }
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(count)) => {
            tracing::warn!(
                user_id = %user_id,
                count = count,
                "SSE client lagged, events dropped"
            );
            Some(Ok(Event::default().event("lagged").data(count.to_string())))
        }
        _ => None,
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
