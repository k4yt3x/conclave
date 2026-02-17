#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Client configuration stored in a TOML file.
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Path to the client's local data directory (SQLite DBs, keys, etc.).
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// Path to the client's configuration directory (config.toml, etc.).
    #[serde(default = "default_config_dir")]
    pub config_dir: PathBuf,

    /// Accept invalid TLS certificates (e.g., self-signed). Default: false.
    #[serde(default)]
    pub accept_invalid_certs: bool,
}

fn default_data_dir() -> PathBuf {
    // $CONCLAVE_DATA_DIR takes top priority.
    if let Ok(dir) = std::env::var("CONCLAVE_DATA_DIR") {
        return PathBuf::from(dir);
    }

    // Fall back to XDG data directory (respects $XDG_DATA_HOME).
    directories::ProjectDirs::from("", "", "conclave")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".conclave"))
}

fn default_config_dir() -> PathBuf {
    // $CONCLAVE_CONFIG_DIR takes top priority.
    if let Ok(dir) = std::env::var("CONCLAVE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }

    // Fall back to XDG config directory (respects $XDG_CONFIG_HOME).
    directories::ProjectDirs::from("", "", "conclave")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".conclave"))
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            config_dir: default_config_dir(),
            accept_invalid_certs: false,
        }
    }
}

impl ClientConfig {
    /// Load configuration from `<config_dir>/config.toml`.
    ///
    /// Falls back to defaults if the file is missing or malformed.
    pub fn load() -> Self {
        let config_dir = default_config_dir();
        let path = config_dir.join("config.toml");
        if path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Ok(config) = toml::from_str::<Self>(&contents) {
                    return config;
                }
            }
        }
        Self::default()
    }
}

/// Session state persisted between client invocations.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionState {
    pub server_url: Option<String>,
    pub token: Option<String>,
    pub user_id: Option<u64>,
    pub username: Option<String>,
}

impl SessionState {
    pub fn load(data_dir: &PathBuf) -> Self {
        let path = data_dir.join("session.toml");
        if path.exists() {
            let contents = std::fs::read_to_string(&path).unwrap_or_default();
            toml::from_str(&contents).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self, data_dir: &PathBuf) -> crate::error::Result<()> {
        std::fs::create_dir_all(data_dir)?;
        #[cfg(unix)]
        std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700))?;
        let path = data_dir.join("session.toml");
        let contents =
            toml::to_string_pretty(self).map_err(|e| crate::error::Error::Config(e.to_string()))?;
        std::fs::write(&path, contents)?;
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    }
}
