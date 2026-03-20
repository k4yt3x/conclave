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

    #[error("forbidden: {0}")]
    Forbidden(String),

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
            code: ErrorCode::ErrAuthTokenExpired.into(),
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
            code: ErrorCode::ErrGroupNotMember.into(),
        }
    }

    pub fn not_admin(message: impl Into<String>) -> Self {
        Error::Unauthorized {
            message: message.into(),
            code: ErrorCode::ErrGroupNotAdmin.into(),
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
                    ErrorCode::ErrUnspecified,
                )
            }
            Error::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".to_string(),
                    ErrorCode::ErrUnspecified,
                )
            }
            Error::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                msg.clone(),
                ErrorCode::ErrResourceNotFound,
            ),
            Error::Conflict(msg) => (
                StatusCode::CONFLICT,
                msg.clone(),
                ErrorCode::ErrResourceConflict,
            ),
            Error::Unauthorized { message, code } => (
                StatusCode::UNAUTHORIZED,
                message.clone(),
                ErrorCode::try_from(*code).unwrap_or(ErrorCode::ErrUnspecified),
            ),
            Error::Forbidden(msg) => (
                StatusCode::FORBIDDEN,
                msg.clone(),
                ErrorCode::ErrResourceForbidden,
            ),
            Error::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                msg.clone(),
                ErrorCode::ErrInputBadRequest,
            ),
            Error::Validation(msg) => (
                StatusCode::BAD_REQUEST,
                msg.clone(),
                ErrorCode::ErrInputValidation,
            ),
            Error::ProtobufDecode(_) => (
                StatusCode::BAD_REQUEST,
                "invalid request body".to_string(),
                ErrorCode::ErrInputBadRequest,
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
