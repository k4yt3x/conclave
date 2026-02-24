use std::hash::{Hash, Hasher};
use std::time::Duration;

use futures_util::StreamExt;
use iced::Subscription;
use reqwest_eventsource::{Event as EsEvent, EventSource};

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// SSE event updates from the server.
#[derive(Debug, Clone)]
pub enum SseUpdate {
    Connected,
    Connecting,
    Disconnected,
    NewMessage {
        group_id: i64,
    },
    Welcome,
    GroupUpdate,
    MemberRemoved {
        group_id: i64,
        username: String,
    },
    IdentityReset {
        group_id: i64,
        username: String,
    },
    InviteReceived {
        invite_id: i64,
        group_id: i64,
        group_name: String,
        group_alias: String,
        inviter_username: String,
    },
    InviteDeclined {
        group_id: i64,
        declined_username: String,
    },
}

/// State key for the SSE subscription. Keyed by token so the subscription
/// restarts if the token changes.
struct SseState {
    base_url: String,
    token: String,
    accept_invalid_certs: bool,
}

impl PartialEq for SseState {
    fn eq(&self, other: &Self) -> bool {
        self.token == other.token && self.base_url == other.base_url
    }
}

impl Hash for SseState {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.token.hash(state);
        self.base_url.hash(state);
    }
}

/// Create an SSE subscription that connects to the server's event stream.
pub fn sse(base_url: String, token: String, accept_invalid_certs: bool) -> Subscription<SseUpdate> {
    Subscription::run_with(
        SseState {
            base_url,
            token,
            accept_invalid_certs,
        },
        |state: &SseState| {
            sse_stream(
                state.base_url.clone(),
                state.token.clone(),
                state.accept_invalid_certs,
            )
        },
    )
}

fn sse_stream(
    base_url: String,
    token: String,
    accept_invalid_certs: bool,
) -> impl futures_util::Stream<Item = SseUpdate> {
    async_stream::stream! {
        let url = format!("{base_url}/api/v1/events");
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(accept_invalid_certs)
            .build()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "SSE HTTP client build failed, using default");
                reqwest::Client::new()
            });

        loop {
            yield SseUpdate::Connecting;

            let builder = client
                .get(&url)
                .header("Authorization", format!("Bearer {token}"));

            let mut es = match EventSource::new(builder) {
                Ok(es) => es,
                Err(_) => {
                    yield SseUpdate::Disconnected;
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
            };

            while let Some(event) = es.next().await {
                match event {
                    Ok(EsEvent::Open) => {
                        yield SseUpdate::Connected;
                    }
                    Ok(EsEvent::Message(msg)) => {
                        if let Some(update) = decode_sse_event(&msg.data) {
                            yield update;
                        }
                    }
                    Err(_) => {
                        yield SseUpdate::Disconnected;
                        break;
                    }
                }
            }

            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    }
}

fn decode_sse_event(hex_data: &str) -> Option<SseUpdate> {
    let event = conclave_lib::operations::decode_sse_event(hex_data).ok()?;

    match event {
        conclave_lib::operations::SseEvent::NewMessage { group_id } => {
            Some(SseUpdate::NewMessage { group_id })
        }
        conclave_lib::operations::SseEvent::Welcome { .. } => Some(SseUpdate::Welcome),
        conclave_lib::operations::SseEvent::GroupUpdate { .. } => Some(SseUpdate::GroupUpdate),
        conclave_lib::operations::SseEvent::MemberRemoved {
            group_id,
            removed_username,
        } => Some(SseUpdate::MemberRemoved {
            group_id,
            username: removed_username,
        }),
        conclave_lib::operations::SseEvent::IdentityReset { group_id, username } => {
            Some(SseUpdate::IdentityReset { group_id, username })
        }
        conclave_lib::operations::SseEvent::InviteReceived {
            invite_id,
            group_id,
            group_name,
            group_alias,
            inviter_username,
        } => Some(SseUpdate::InviteReceived {
            invite_id,
            group_id,
            group_name,
            group_alias,
            inviter_username,
        }),
        conclave_lib::operations::SseEvent::InviteDeclined {
            group_id,
            declined_username,
        } => Some(SseUpdate::InviteDeclined {
            group_id,
            declined_username,
        }),
    }
}
