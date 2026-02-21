use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use prost::Message;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("protobuf decode error: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, client_message) = match &self {
            Error::Database(e) => {
                tracing::error!(error = %e, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
            Error::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                )
            }
            Error::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Error::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            Error::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            Error::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Error::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Error::ProtobufDecode(_) => {
                (StatusCode::BAD_REQUEST, "invalid request body".to_string())
            }
        };

        let error_resp = conclave_proto::ErrorResponse {
            message: client_message,
        };

        let mut body = Vec::new();
        if let Err(error) = error_resp.encode(&mut body) {
            tracing::error!(%error, "failed to encode error response");
        }

        (
            status,
            [(axum::http::header::CONTENT_TYPE, "application/x-protobuf")],
            body,
        )
            .into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;
