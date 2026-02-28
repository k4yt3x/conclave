use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Address to listen on (default: "0.0.0.0").
    #[serde(default = "default_listen_address")]
    pub listen_address: String,

    /// Port to listen on. Defaults to 8443 when TLS is configured, 8080 otherwise.
    pub listen_port: Option<u16>,

    /// Path to the SQLite database file.
    #[serde(default = "default_db_path")]
    pub database_path: PathBuf,

    /// Session token lifetime in seconds (default: 7 days).
    #[serde(default = "default_token_ttl")]
    pub token_ttl_seconds: i64,

    /// Pending invite expiration in seconds (default: 7 days).
    #[serde(default = "default_invite_ttl")]
    pub invite_ttl_seconds: i64,

    /// Server-wide maximum message age. Accepts duration format: `-1` (disabled, default),
    /// `0` (delete after all members fetch), or `<number><unit>` (e.g., `30d`, `1w`).
    #[serde(default = "default_message_retention")]
    pub message_retention: String,

    /// Interval between message cleanup runs. Accepts duration format (e.g., `1h`, `30s`).
    /// Default: `1h`.
    #[serde(default = "default_cleanup_interval")]
    pub cleanup_interval: String,

    /// Path to the TLS certificate file (PEM format).
    /// If both `tls_cert_path` and `tls_key_path` are set, the server serves HTTPS.
    pub tls_cert_path: Option<PathBuf>,

    /// Path to the TLS private key file (PEM format).
    pub tls_key_path: Option<PathBuf>,
}

fn default_listen_address() -> String {
    "0.0.0.0".to_string()
}

fn default_db_path() -> PathBuf {
    PathBuf::from("conclave.db")
}

fn default_token_ttl() -> i64 {
    7 * 24 * 60 * 60 // 7 days
}

fn default_invite_ttl() -> i64 {
    7 * 24 * 60 * 60 // 7 days
}

fn default_message_retention() -> String {
    "-1".to_string()
}

fn default_cleanup_interval() -> String {
    "1h".to_string()
}

impl ServerConfig {
    /// Parse the `message_retention` config string into seconds.
    pub fn message_retention_seconds(&self) -> i64 {
        crate::duration::parse_duration(&self.message_retention).unwrap_or(-1)
    }

    /// Parse the `cleanup_interval` config string into seconds (minimum 1).
    pub fn cleanup_interval_seconds(&self) -> u64 {
        crate::duration::parse_duration(&self.cleanup_interval)
            .unwrap_or(3600)
            .max(1) as u64
    }

    /// Returns the socket address string (e.g., "0.0.0.0:8443") by combining listen_address
    /// and listen_port. When listen_port is not set, defaults to 8443 for TLS or 8080 for
    /// plain HTTP.
    pub fn socket_address(&self) -> String {
        let port = self.listen_port.unwrap_or_else(|| {
            if self.tls_cert_path.is_some() && self.tls_key_path.is_some() {
                8443
            } else {
                8080
            }
        });
        format!("{}:{}", self.listen_address, port)
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_address: default_listen_address(),
            listen_port: None,
            database_path: default_db_path(),
            token_ttl_seconds: default_token_ttl(),
            invite_ttl_seconds: default_invite_ttl(),
            message_retention: default_message_retention(),
            cleanup_interval: default_cleanup_interval(),
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}
