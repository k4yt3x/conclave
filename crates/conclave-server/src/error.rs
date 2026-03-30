use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use prost::Message;

use conclave_proto::ErrorCode;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("unauthorized: {message}")]
    Unauthorized { message: String, code: i32 },

    #[error("forbidden: {message}")]
    Forbidden { message: String, code: i32 },

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("protobuf decode error: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),
}

impl Error {
    pub fn token_expired(message: impl Into<String>) -> Self {
        Error::Unauthorized {
            message: message.into(),
            code: ErrorCode::AuthTokenExpired.into(),
        }
    }

    pub fn auth_misconfigured(message: impl Into<String>, code: ErrorCode) -> Self {
        Error::Unauthorized {
            message: message.into(),
            code: code.into(),
        }
    }

    pub fn not_member(message: impl Into<String>) -> Self {
        Error::Unauthorized {
            message: message.into(),
            code: ErrorCode::GroupNotMember.into(),
        }
    }

    pub fn not_admin(message: impl Into<String>) -> Self {
        Error::Unauthorized {
            message: message.into(),
            code: ErrorCode::GroupNotAdmin.into(),
        }
    }

    pub fn not_public(message: impl Into<String>) -> Self {
        Error::Forbidden {
            message: message.into(),
            code: ErrorCode::GroupNotPublic.into(),
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, client_message, error_code) = match &self {
            Error::Database(e) => {
                tracing::error!(error = %e, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                    ErrorCode::Unspecified,
                )
            }
            Error::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                    ErrorCode::Unspecified,
                )
            }
            Error::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                msg.clone(),
                ErrorCode::ResourceNotFound,
            ),
            Error::Conflict(msg) => (
                StatusCode::CONFLICT,
                msg.clone(),
                ErrorCode::ResourceConflict,
            ),
            Error::Unauthorized { message, code } => (
                StatusCode::UNAUTHORIZED,
                message.clone(),
                ErrorCode::try_from(*code).unwrap_or(ErrorCode::Unspecified),
            ),
            Error::Forbidden { message, code } => (
                StatusCode::FORBIDDEN,
                message.clone(),
                ErrorCode::try_from(*code).unwrap_or(ErrorCode::ResourceForbidden),
            ),
            Error::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                msg.clone(),
                ErrorCode::InputBadRequest,
            ),
            Error::Validation(msg) => (
                StatusCode::BAD_REQUEST,
                msg.clone(),
                ErrorCode::InputValidation,
            ),
            Error::ProtobufDecode(_) => (
                StatusCode::BAD_REQUEST,
                "invalid request body".to_string(),
                ErrorCode::InputBadRequest,
            ),
        };

        let error_resp = conclave_proto::ErrorResponse {
            message: client_message,
            error_code: error_code.into(),
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
