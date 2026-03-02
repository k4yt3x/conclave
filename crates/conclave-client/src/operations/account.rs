use std::collections::HashMap;
use std::path::Path;

use crate::api::{ApiClient, normalize_server_url};
use crate::config::{SessionState, generate_initial_key_packages};
use crate::error::{Error, Result};
use crate::mls::MlsManager;

use super::{ResetResult, load_rooms};

/// Result of a successful registration or login.
#[derive(Debug, Clone)]
pub struct AuthResult {
    pub server_url: String,
    pub token: String,
    pub user_id: i64,
    pub username: String,
    pub key_packages_uploaded: usize,
}

impl AuthResult {
    /// Build an authenticated `ApiClient` from this result.
    pub fn into_api_client(&self, accept_invalid_certs: bool) -> ApiClient {
        let mut api = ApiClient::new(&self.server_url, accept_invalid_certs);
        api.set_token(self.token.clone());
        api
    }

    /// Save the session state to disk.
    pub fn save_session(&self, data_dir: &Path) -> Result<()> {
        let session = SessionState {
            server_url: Some(self.server_url.clone()),
            token: Some(self.token.clone()),
            user_id: Some(self.user_id),
            username: Some(self.username.clone()),
        };
        session.save(data_dir)
    }
}

/// Register a new account, log in, initialize MLS, and upload key packages.
pub async fn register_and_login(
    server_url: &str,
    username: &str,
    password: &str,
    registration_token: Option<&str>,
    accept_invalid_certs: bool,
    data_dir: &Path,
) -> Result<AuthResult> {
    let server_url = normalize_server_url(server_url);
    let api = ApiClient::new(&server_url, accept_invalid_certs);

    let register_response = api
        .register(username, password, None, registration_token)
        .await?;
    let user_id = register_response.user_id;

    let login_response = api.login(username, password).await?;
    let canonical_username = if login_response.username.is_empty() {
        username.to_string()
    } else {
        login_response.username
    };

    let mut auth_api = ApiClient::new(&server_url, accept_invalid_certs);
    auth_api.set_token(login_response.token.clone());

    let count = initialize_mls_and_upload_key_packages(&auth_api, data_dir, user_id).await?;

    Ok(AuthResult {
        server_url,
        token: login_response.token,
        user_id,
        username: canonical_username,
        key_packages_uploaded: count,
    })
}

/// Log in to an existing account, initialize MLS, and upload key packages.
pub async fn login(
    server_url: &str,
    username: &str,
    password: &str,
    accept_invalid_certs: bool,
    data_dir: &Path,
) -> Result<AuthResult> {
    let server_url = normalize_server_url(server_url);
    let api = ApiClient::new(&server_url, accept_invalid_certs);

    let login_response = api.login(username, password).await?;
    let canonical_username = if login_response.username.is_empty() {
        username.to_string()
    } else {
        login_response.username
    };

    let mut auth_api = ApiClient::new(&server_url, accept_invalid_certs);
    auth_api.set_token(login_response.token.clone());

    let count =
        initialize_mls_and_upload_key_packages(&auth_api, data_dir, login_response.user_id).await?;

    Ok(AuthResult {
        server_url,
        token: login_response.token,
        user_id: login_response.user_id,
        username: canonical_username,
        key_packages_uploaded: count,
    })
}

/// Initialize the local MLS provider and generate+upload the initial set of key
/// packages to the server. Call this after a successful login or registration.
///
/// Creates `data_dir` if it does not exist. Returns the number of key packages
/// uploaded.
pub async fn initialize_mls_and_upload_key_packages(
    api: &ApiClient,
    data_dir: &Path,
    user_id: i64,
) -> Result<usize> {
    std::fs::create_dir_all(data_dir)?;

    let data_dir = data_dir.to_path_buf();
    let (entries, fingerprint) = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        let entries = generate_initial_key_packages(&mls)?;
        let fingerprint = mls.signing_key_fingerprint();
        Ok::<_, Error>((entries, fingerprint))
    })
    .await
    .map_err(super::map_join_error)??;

    let count = entries.len();
    api.upload_key_packages(entries, &fingerprint).await?;
    Ok(count)
}

/// Delete the user's account on the server and wipe all local data.
pub async fn delete_account(api: &ApiClient, password: &str, data_dir: &Path) -> Result<()> {
    api.delete_account(password).await?;

    if data_dir.exists() {
        std::fs::remove_dir_all(data_dir)?;
    }

    Ok(())
}

/// Reset the account: wipe all local MLS state, regenerate identity and key
/// packages, then rejoin each group via external commit.
///
/// Groups are discovered from the server (not from local state), so this works
/// even when the user has lost their local data directory.
pub async fn reset_account(api: &ApiClient, data_dir: &Path, user_id: i64) -> Result<ResetResult> {
    // Step 1: Fetch group list from the server.
    let rooms = load_rooms(api).await?;
    let groups_to_rejoin: Vec<i64> = rooms.iter().map(|r| r.group_id).collect();
    let total_groups = groups_to_rejoin.len();

    // Step 2: Collect old leaf indices before wiping state (best-effort;
    // the mapping and MLS state may be missing after data loss).
    let old_indices: HashMap<i64, Option<u32>> = {
        let data_dir = data_dir.to_path_buf();
        let groups = groups_to_rejoin.clone();
        let group_mapping = crate::config::build_group_mapping(&rooms, &data_dir);

        tokio::task::spawn_blocking(move || {
            let mls = match MlsManager::new(&data_dir, user_id) {
                Ok(mls) => mls,
                Err(_) => return Ok(HashMap::new()),
            };
            let mut indices = HashMap::new();
            for server_id in &groups {
                if let Some(mls_id) = group_mapping.get(server_id) {
                    let index = mls.find_member_index(mls_id, user_id).ok().flatten();
                    indices.insert(*server_id, index);
                }
            }
            Ok::<_, Error>(indices)
        })
        .await
        .map_err(super::map_join_error)?
    }?;

    // Step 3: Notify server to clear our key packages.
    api.reset_account().await?;

    // Step 4: Wipe local MLS state.
    {
        let data_dir = data_dir.to_path_buf();

        tokio::task::spawn_blocking(move || {
            let mls = match MlsManager::new(&data_dir, user_id) {
                Ok(mls) => mls,
                Err(_) => return Ok(()),
            };
            mls.wipe_local_state()
        })
        .await
        .map_err(super::map_join_error)?
    }?;

    // Step 5: Regenerate identity and upload new key packages.
    {
        let data_dir = data_dir.to_path_buf();

        let (entries, fingerprint) = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            let entries = generate_initial_key_packages(&mls)?;
            let fingerprint = mls.signing_key_fingerprint();
            Ok::<_, Error>((entries, fingerprint))
        })
        .await
        .map_err(super::map_join_error)??;

        api.upload_key_packages(entries, &fingerprint).await?;
    }

    // Step 6: Rejoin each group via external commit.
    let (new_group_mapping, rejoin_count, errors) =
        rejoin_groups_via_external_commit(api, &groups_to_rejoin, &old_indices, data_dir, user_id)
            .await?;

    Ok(ResetResult {
        new_group_mapping,
        rejoin_count,
        total_groups,
        errors,
    })
}

async fn rejoin_groups_via_external_commit(
    api: &ApiClient,
    groups: &[i64],
    old_indices: &HashMap<i64, Option<u32>>,
    data_dir: &Path,
    user_id: i64,
) -> Result<(HashMap<i64, String>, usize, Vec<String>)> {
    let mut new_group_mapping = HashMap::new();
    let mut errors = Vec::new();
    let mut rejoin_count = 0;

    for server_group_id in groups {
        let server_group_id = *server_group_id;

        let group_info_response = match api.get_group_info(server_group_id).await {
            Ok(response) => response,
            Err(error) => {
                errors.push(format!(
                    "Failed to get group info for {server_group_id}: {error}"
                ));
                continue;
            }
        };

        let old_index = old_indices.get(&server_group_id).copied().flatten();
        let group_info_bytes = group_info_response.group_info.clone();

        let data_dir = data_dir.to_path_buf();

        let rejoin_result = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            mls.external_rejoin_group(&group_info_bytes, old_index)
        })
        .await
        .map_err(super::map_join_error)?;

        match rejoin_result {
            Ok((new_mls_id, commit_bytes)) => {
                if let Err(error) = api
                    .external_join(server_group_id, commit_bytes, &new_mls_id)
                    .await
                {
                    errors.push(format!("Failed to rejoin {server_group_id}: {error}"));
                    continue;
                }
                new_group_mapping.insert(server_group_id, new_mls_id);
                rejoin_count += 1;
            }
            Err(error) => {
                errors.push(format!(
                    "Failed external commit for {server_group_id}: {error}"
                ));
            }
        }
    }

    Ok((new_group_mapping, rejoin_count, errors))
}
