use std::hash::{Hash, Hasher};
use std::time::Duration;

use futures_util::StreamExt;
use iced::Subscription;
use prost::Message;
use reqwest_eventsource::{Event as EsEvent, EventSource};

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// SSE event updates from the server.
#[derive(Debug, Clone)]
pub enum SseUpdate {
    Connected,
    Connecting,
    Disconnected,
    NewMessage { group_id: String },
    Welcome,
    GroupUpdate,
    MemberRemoved { group_id: String, username: String },
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
            .unwrap_or_default();

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
    let bytes = hex::decode(hex_data).ok()?;
    let event = conclave_proto::ServerEvent::decode(bytes.as_slice()).ok()?;

    match event.event? {
        conclave_proto::server_event::Event::NewMessage(msg) => Some(SseUpdate::NewMessage {
            group_id: msg.group_id,
        }),
        conclave_proto::server_event::Event::Welcome(_) => Some(SseUpdate::Welcome),
        conclave_proto::server_event::Event::GroupUpdate(_) => Some(SseUpdate::GroupUpdate),
        conclave_proto::server_event::Event::MemberRemoved(r) => Some(SseUpdate::MemberRemoved {
            group_id: r.group_id,
            username: r.removed_username,
        }),
    }
}
