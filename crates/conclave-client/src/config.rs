use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Client configuration stored in a TOML file.
#[derive(Debug, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Server base URL (e.g., "http://127.0.0.1:8443").
    pub server_url: String,

    /// Path to the client's local data directory (SQLite DBs, keys, etc.).
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
}

fn default_data_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "conclave")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".conclave"))
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_url: "http://127.0.0.1:8443".to_string(),
            data_dir: default_data_dir(),
        }
    }
}

/// Session state persisted between client invocations.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SessionState {
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
        let path = data_dir.join("session.toml");
        let contents =
            toml::to_string_pretty(self).map_err(|e| crate::error::Error::Config(e.to_string()))?;
        std::fs::write(path, contents)?;
        Ok(())
    }
}
