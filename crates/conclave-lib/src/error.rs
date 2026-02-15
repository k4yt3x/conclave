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

    #[error("server error ({status}): {message}")]
    Server { status: u16, message: String },

    #[error("MLS error: {0}")]
    Mls(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
