use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Address to bind the server to (e.g., "0.0.0.0:8443").
    #[serde(default = "default_bind_address")]
    pub bind_address: String,

    /// Path to the SQLite database file.
    #[serde(default = "default_db_path")]
    pub database_path: PathBuf,

    /// Session token lifetime in seconds (default: 7 days).
    #[serde(default = "default_token_ttl")]
    pub token_ttl_seconds: i64,

    /// Path to the TLS certificate file (PEM format).
    /// If both `tls_cert_path` and `tls_key_path` are set, the server serves HTTPS.
    pub tls_cert_path: Option<PathBuf>,

    /// Path to the TLS private key file (PEM format).
    pub tls_key_path: Option<PathBuf>,
}

fn default_bind_address() -> String {
    "0.0.0.0:8443".to_string()
}

fn default_db_path() -> PathBuf {
    PathBuf::from("conclave.db")
}

fn default_token_ttl() -> i64 {
    7 * 24 * 60 * 60 // 7 days
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: default_bind_address(),
            database_path: default_db_path(),
            token_ttl_seconds: default_token_ttl(),
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}
