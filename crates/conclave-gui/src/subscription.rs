use std::hash::{Hash, Hasher};
use std::time::Duration;

use futures_util::StreamExt;
use iced::Subscription;
use prost::Message;
use reqwest_eventsource::{Event as EsEvent, EventSource};
use uuid::Uuid;

const RECONNECT_DELAY: Duration = Duration::from_secs(5);

/// SSE event updates from the server.
#[derive(Debug, Clone)]
pub enum SseUpdate {
    Connected,
    Connecting,
    Disconnected,
    Unauthorized,
    NewMessage {
        group_id: Uuid,
    },
    Welcome,
    GroupUpdate,
    MemberRemoved {
        group_id: Uuid,
        removed_user_id: Uuid,
    },
    IdentityReset {
        group_id: Uuid,
        user_id: Uuid,
    },
    InviteReceived {
        invite_id: Uuid,
        group_id: Uuid,
        group_name: String,
        group_alias: String,
        inviter_id: Uuid,
    },
    InviteDeclined {
        group_id: Uuid,
        declined_user_id: Uuid,
    },
    InviteCancelled,
    GroupDeleted {
        group_id: Uuid,
    },
}

/// State key for the SSE subscription. Keyed by token so the subscription
/// restarts if the token changes.
struct SseState {
    base_url: String,
    token: String,
    client: reqwest::Client,
    auth_header: String,
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
pub fn sse(
    base_url: String,
    token: String,
    client: reqwest::Client,
    auth_header: String,
) -> Subscription<SseUpdate> {
    Subscription::run_with(
        SseState {
            base_url,
            token,
            client,
            auth_header,
        },
        |state: &SseState| {
            sse_stream(
                state.base_url.clone(),
                state.token.clone(),
                state.client.clone(),
                state.auth_header.clone(),
            )
        },
    )
}

fn sse_stream(
    base_url: String,
    token: String,
    client: reqwest::Client,
    auth_header: String,
) -> impl futures_util::Stream<Item = SseUpdate> {
    async_stream::stream! {
        let url = format!("{base_url}/api/v1/events");
        let uses_standard = auth_header.eq_ignore_ascii_case(
            reqwest::header::AUTHORIZATION.as_str(),
        );

        loop {
            yield SseUpdate::Connecting;

            let value = if uses_standard {
                format!("Bearer {token}")
            } else {
                token.clone()
            };
            let builder = client.get(&url).header(&auth_header, value);

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
                    Err(error) => {
                        if let reqwest_eventsource::Error::InvalidStatusCode(
                            status, response,
                        ) = error
                        {
                            if status == reqwest::StatusCode::UNAUTHORIZED {
                                let is_token_expired = response
                                    .bytes()
                                    .await
                                    .ok()
                                    .and_then(|body| {
                                        conclave_proto::ErrorResponse::decode(body.as_ref()).ok()
                                    })
                                    .is_some_and(|err| {
                                        err.error_code
                                            == conclave_proto::ErrorCode::ErrAuthTokenExpired as i32
                                    });
                                if is_token_expired {
                                    yield SseUpdate::Unauthorized;
                                    return;
                                }
                            }
                        }
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
    let event = conclave_client::operations::decode_sse_event(hex_data).ok()?;

    match event {
        conclave_client::operations::SseEvent::NewMessage { group_id } => {
            Some(SseUpdate::NewMessage { group_id })
        }
        conclave_client::operations::SseEvent::Welcome { .. } => Some(SseUpdate::Welcome),
        conclave_client::operations::SseEvent::GroupUpdate { .. } => Some(SseUpdate::GroupUpdate),
        conclave_client::operations::SseEvent::MemberRemoved {
            group_id,
            removed_user_id,
        } => Some(SseUpdate::MemberRemoved {
            group_id,
            removed_user_id,
        }),
        conclave_client::operations::SseEvent::IdentityReset { group_id, user_id } => {
            Some(SseUpdate::IdentityReset { group_id, user_id })
        }
        conclave_client::operations::SseEvent::InviteReceived {
            invite_id,
            group_id,
            group_name,
            group_alias,
            inviter_id,
        } => Some(SseUpdate::InviteReceived {
            invite_id,
            group_id,
            group_name,
            group_alias,
            inviter_id,
        }),
        conclave_client::operations::SseEvent::InviteDeclined {
            group_id,
            declined_user_id,
        } => Some(SseUpdate::InviteDeclined {
            group_id,
            declined_user_id,
        }),
        conclave_client::operations::SseEvent::InviteCancelled { .. } => {
            Some(SseUpdate::InviteCancelled)
        }
        conclave_client::operations::SseEvent::GroupDeleted { group_id } => {
            Some(SseUpdate::GroupDeleted { group_id })
        }
    }
}
