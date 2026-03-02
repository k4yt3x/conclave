use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    /// Address to listen on (default: "0.0.0.0").
    #[serde(default = "default_listen_address")]
    pub listen_address: String,

    /// Port to listen on. Defaults to 8443 when TLS is configured, 8080 otherwise.
    pub listen_port: Option<u16>,

    /// Path to the SQLite database file (default: "conclave.db").
    #[serde(default = "default_db_path")]
    pub database_path: PathBuf,

    /// Session token lifetime in seconds (default: 604800 = 7 days).
    #[serde(default = "default_token_ttl")]
    pub token_ttl_seconds: i64,

    /// Pending invite expiration in seconds (default: 604800 = 7 days).
    #[serde(default = "default_invite_ttl")]
    pub invite_ttl_seconds: i64,

    /// Global message retention policy. Determines the maximum age of messages stored
    /// on the server. Special values: "-1" (default) disables retention (messages kept
    /// indefinitely), "0" deletes messages after all group members have fetched them.
    /// Duration format: "15s", "2h", "7d", "4w", "1m" (30d), "1y" (365d).
    #[serde(default = "default_message_retention")]
    pub message_retention: String,

    /// Interval between cleanup runs for expired sessions, invites, and messages.
    /// Duration format (e.g., "1h", "30s"). Default: "1h".
    #[serde(default = "default_cleanup_interval")]
    pub cleanup_interval: String,

    /// Whether public (open) registration is enabled (default: true).
    /// When true, anyone can register and the registration token is ignored.
    /// When false, registration requires a valid registration_token (if set)
    /// or is entirely disabled (if no token is configured).
    #[serde(default = "default_registration_enabled")]
    pub registration_enabled: bool,

    /// Registration token for invite-only registration. Only checked when
    /// registration_enabled is false. Must contain only ASCII letters, digits,
    /// underscores, and hyphens ([a-zA-Z0-9_-]).
    pub registration_token: Option<String>,

    /// Path to the TLS certificate file (PEM format).
    /// If both are set, the server listens with TLS (HTTPS) on port 8443 by default.
    /// If omitted, the server listens on plain HTTP on port 8080 by default.
    /// When running behind a TLS-terminating reverse proxy, leave these unset.
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
    30 * 24 * 60 * 60 // 30 days
}

fn default_invite_ttl() -> i64 {
    30 * 24 * 60 * 60 // 30 days
}

fn default_message_retention() -> String {
    "-1".to_string()
}

fn default_cleanup_interval() -> String {
    "1h".to_string()
}

fn default_registration_enabled() -> bool {
    true
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

    /// Validate configuration values. Returns an error message if invalid.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if let Some(ref token) = self.registration_token {
            if token.is_empty() {
                return Err("registration_token must not be empty when set".into());
            }
            if !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(
                    "registration_token must contain only ASCII letters, digits, underscores, and hyphens".into(),
                );
            }
        }
        Ok(())
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
            registration_enabled: default_registration_enabled(),
            registration_token: None,
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}
