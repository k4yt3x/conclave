/// Format an error and its full cause chain into a single string.
fn format_error_chain(err: &dyn std::error::Error) -> String {
    let mut msg = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        msg.push_str(": ");
        msg.push_str(&cause.to_string());
        source = cause.source();
    }
    msg
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP error: {}", format_error_chain(.0))]
    Http(#[from] reqwest::Error),

    #[error("protobuf decode error: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),

    #[error("server error ({status}/{error_code}): {message}")]
    Server {
        status: u16,
        message: String,
        error_code: i32,
    },

    #[error("MLS error: {0}")]
    Mls(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("invalid UUID: {0}")]
    InvalidUuid(#[from] uuid::Error),

    #[error("another conclave instance is already running")]
    InstanceAlreadyRunning,

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Check if this error represents an HTTP 401 Unauthorized response.
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Error::Server { status: 401, .. })
    }

    /// Check if this error indicates the session token is invalid or expired.
    /// Returns false for auth configuration errors (wrong header name, missing
    /// Bearer prefix) which are also 401 but not session issues.
    pub fn is_session_expired(&self) -> bool {
        matches!(
            self,
            Error::Server { error_code, .. }
                if *error_code == conclave_proto::ErrorCode::AuthTokenExpired as i32
        )
    }
}

pub type Result<T> = std::result::Result<T, Error>;
