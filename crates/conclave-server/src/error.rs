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

    #[error("internal error: {0}")]
    Internal(String),

    #[error("protobuf decode error: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Error::Database(_) | Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Error::BadRequest(_) => StatusCode::BAD_REQUEST,
            Error::ProtobufDecode(_) => StatusCode::BAD_REQUEST,
        };

        let error_resp = conclave_proto::ErrorResponse {
            message: self.to_string(),
        };

        let mut body = Vec::new();
        error_resp.encode(&mut body).unwrap();

        (
            status,
            [(axum::http::header::CONTENT_TYPE, "application/x-protobuf")],
            body,
        )
            .into_response()
    }
}

pub type Result<T> = std::result::Result<T, Error>;
