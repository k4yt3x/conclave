#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Lib(#[from] conclave_lib::error::Error),

    #[error("terminal error: {0}")]
    Terminal(String),
}

pub type Result<T> = std::result::Result<T, Error>;
