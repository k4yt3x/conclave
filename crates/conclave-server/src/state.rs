use tokio::sync::broadcast;

use crate::config::ServerConfig;
use crate::db::Database;

/// SSE event sent to connected clients.
#[derive(Debug, Clone)]
pub struct SseEvent {
    /// The protobuf-encoded ServerEvent bytes.
    pub data: Vec<u8>,
    /// Target user IDs that should receive this event.
    pub target_user_ids: Vec<i64>,
}

/// Shared application state.
pub struct AppState {
    pub db: Database,
    pub config: ServerConfig,
    pub sse_tx: broadcast::Sender<SseEvent>,
}

impl AppState {
    pub fn new(db: Database, config: ServerConfig) -> Self {
        let (sse_tx, _) = broadcast::channel(1024);
        Self { db, config, sse_tx }
    }
}
