use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mls::MlsManager;

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

pub fn load_group_mapping(data_dir: &Path) -> HashMap<String, String> {
    let path = data_dir.join("group_mapping.toml");
    if path.exists() {
        let contents = std::fs::read_to_string(&path).unwrap_or_default();
        toml::from_str(&contents).unwrap_or_default()
    } else {
        HashMap::new()
    }
}

pub fn save_group_mapping(data_dir: &Path, mapping: &HashMap<String, String>) {
    let path = data_dir.join("group_mapping.toml");
    if let Ok(contents) = toml::to_string_pretty(mapping) {
        let _ = std::fs::write(&path, contents);
        #[cfg(unix)]
        {
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

pub fn generate_initial_key_packages(
    mls: &MlsManager,
) -> crate::error::Result<Vec<(Vec<u8>, bool)>> {
    let mut entries = Vec::with_capacity(6);
    let last_resort = mls.generate_last_resort_key_package()?;
    entries.push((last_resort, true));
    for key_package in mls.generate_key_packages(5)? {
        entries.push((key_package, false));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_client_config_default() {
        let config = ClientConfig::default();
        assert!(!config.accept_invalid_certs);
        assert!(!config.data_dir.as_os_str().is_empty());
        assert!(!config.config_dir.as_os_str().is_empty());
    }

    #[test]
    fn test_session_state_default() {
        let state = SessionState::default();
        assert!(state.server_url.is_none());
        assert!(state.token.is_none());
        assert!(state.user_id.is_none());
        assert!(state.username.is_none());
    }

    #[test]
    fn test_session_state_save_and_load() {
        let dir = TempDir::new().unwrap();
        let state = SessionState {
            server_url: Some("https://example.com".into()),
            token: Some("tok123".into()),
            user_id: Some(42),
            username: Some("alice".into()),
        };
        state.save(&dir.path().to_path_buf()).unwrap();
        let loaded = SessionState::load(&dir.path().to_path_buf());
        assert_eq!(loaded.server_url.as_deref(), Some("https://example.com"));
        assert_eq!(loaded.token.as_deref(), Some("tok123"));
        assert_eq!(loaded.user_id, Some(42));
        assert_eq!(loaded.username.as_deref(), Some("alice"));
    }

    #[test]
    fn test_session_state_load_missing_file() {
        let dir = TempDir::new().unwrap();
        let loaded = SessionState::load(&dir.path().to_path_buf());
        assert!(loaded.server_url.is_none());
        assert!(loaded.token.is_none());
    }

    #[test]
    fn test_session_state_overwrite() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let state1 = SessionState {
            server_url: Some("https://first.com".into()),
            token: Some("tok1".into()),
            user_id: Some(1),
            username: Some("alice".into()),
        };
        state1.save(&path).unwrap();

        let state2 = SessionState {
            server_url: Some("https://second.com".into()),
            token: Some("tok2".into()),
            user_id: Some(2),
            username: Some("bob".into()),
        };
        state2.save(&path).unwrap();

        let loaded = SessionState::load(&path);
        assert_eq!(loaded.server_url.as_deref(), Some("https://second.com"));
        assert_eq!(loaded.username.as_deref(), Some("bob"));
    }

    #[test]
    fn test_session_state_creates_nested_dirs() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a").join("b").join("c");
        let state = SessionState {
            server_url: Some("https://example.com".into()),
            ..Default::default()
        };
        state.save(&nested).unwrap();
        let loaded = SessionState::load(&nested);
        assert_eq!(loaded.server_url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn test_session_state_partial_data() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let state = SessionState {
            server_url: Some("https://example.com".into()),
            token: None,
            user_id: None,
            username: None,
        };
        state.save(&path).unwrap();
        let loaded = SessionState::load(&path);
        assert_eq!(loaded.server_url.as_deref(), Some("https://example.com"));
        assert!(loaded.token.is_none());
        assert!(loaded.user_id.is_none());
        assert!(loaded.username.is_none());
    }

    #[test]
    fn test_group_mapping_empty() {
        let dir = TempDir::new().unwrap();
        let mapping = load_group_mapping(dir.path());
        assert!(mapping.is_empty());
    }

    #[test]
    fn test_group_mapping_save_and_load() {
        let dir = TempDir::new().unwrap();
        let mut mapping = HashMap::new();
        mapping.insert("server-uuid-1".into(), "mls-group-1".into());
        mapping.insert("server-uuid-2".into(), "mls-group-2".into());
        save_group_mapping(dir.path(), &mapping);
        let loaded = load_group_mapping(dir.path());
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get("server-uuid-1").unwrap(), "mls-group-1");
        assert_eq!(loaded.get("server-uuid-2").unwrap(), "mls-group-2");
    }

    #[test]
    fn test_group_mapping_overwrite() {
        let dir = TempDir::new().unwrap();
        let mut map1 = HashMap::new();
        map1.insert("key1".into(), "val1".into());
        save_group_mapping(dir.path(), &map1);

        let mut map2 = HashMap::new();
        map2.insert("key2".into(), "val2".into());
        save_group_mapping(dir.path(), &map2);

        let loaded = load_group_mapping(dir.path());
        assert_eq!(loaded.len(), 1);
        assert!(loaded.contains_key("key2"));
        assert!(!loaded.contains_key("key1"));
    }

    #[test]
    fn test_generate_initial_key_packages_count() {
        let dir = TempDir::new().unwrap();
        let mls = MlsManager::new(dir.path(), "testuser").unwrap();
        let entries = generate_initial_key_packages(&mls).unwrap();
        assert_eq!(entries.len(), 6);

        let last_resort_count = entries.iter().filter(|(_, lr)| *lr).count();
        assert_eq!(last_resort_count, 1);

        let regular_count = entries.iter().filter(|(_, lr)| !*lr).count();
        assert_eq!(regular_count, 5);

        // First entry should be the last-resort package
        assert!(entries[0].1);
    }

    #[cfg(unix)]
    #[test]
    fn test_session_state_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let state = SessionState {
            server_url: Some("https://example.com".into()),
            ..Default::default()
        };
        state.save(&path).unwrap();

        let session_path = path.join("session.toml");
        let perms = std::fs::metadata(&session_path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);

        let dir_perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(dir_perms.mode() & 0o777, 0o700);
    }

    #[test]
    fn test_group_mapping_empty_values() {
        let dir = TempDir::new().unwrap();
        let mut mapping = HashMap::new();
        mapping.insert("key1".into(), "".into());
        mapping.insert("key2".into(), "".into());
        save_group_mapping(dir.path(), &mapping);
        let loaded = load_group_mapping(dir.path());
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get("key1").unwrap(), "");
        assert_eq!(loaded.get("key2").unwrap(), "");
    }

    #[test]
    fn test_group_mapping_many_entries() {
        let dir = TempDir::new().unwrap();
        let mut mapping = HashMap::new();
        for i in 0..100 {
            mapping.insert(format!("uuid-{i}"), format!("mls-group-{i}"));
        }
        save_group_mapping(dir.path(), &mapping);
        let loaded = load_group_mapping(dir.path());
        assert_eq!(loaded.len(), 100);
        assert_eq!(loaded.get("uuid-0").unwrap(), "mls-group-0");
        assert_eq!(loaded.get("uuid-50").unwrap(), "mls-group-50");
        assert_eq!(loaded.get("uuid-99").unwrap(), "mls-group-99");
    }

    #[test]
    fn test_session_state_load_malformed_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "this is not valid [[[ toml {{{{").unwrap();
        let loaded = SessionState::load(&dir.path().to_path_buf());
        assert!(loaded.server_url.is_none());
        assert!(loaded.token.is_none());
        assert!(loaded.user_id.is_none());
        assert!(loaded.username.is_none());
    }

    #[test]
    fn test_client_config_load_missing_file() {
        // ClientConfig::load() falls back to defaults when the config file does not exist.
        // We cannot easily redirect the config dir without unsafe env var manipulation,
        // so we verify that load() returns a valid config matching defaults.
        let config = ClientConfig::load();
        let default_config = ClientConfig::default();
        assert_eq!(config.accept_invalid_certs, default_config.accept_invalid_certs);
    }

    #[test]
    fn test_generate_initial_key_packages_structure() {
        let dir = TempDir::new().unwrap();
        let mls = MlsManager::new(dir.path(), "testuser").unwrap();
        let entries = generate_initial_key_packages(&mls).unwrap();
        assert_eq!(entries.len(), 6);

        // All key packages should have non-empty data bytes.
        for (data, _) in &entries {
            assert!(!data.is_empty());
        }

        // First entry should be last-resort.
        assert!(entries[0].1);

        // Remaining 5 entries should be regular (not last-resort).
        for (_, is_last_resort) in &entries[1..] {
            assert!(!is_last_resort);
        }
    }
}
