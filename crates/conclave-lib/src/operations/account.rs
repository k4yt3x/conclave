use std::collections::HashMap;
use std::path::Path;

use crate::api::ApiClient;
use crate::config::generate_initial_key_packages;
use crate::error::{Error, Result};
use crate::mls::MlsManager;

use super::{ResetResult, load_rooms};

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
    let entries = tokio::task::spawn_blocking(move || {
        let mls = MlsManager::new(&data_dir, user_id)?;
        generate_initial_key_packages(&mls)
    })
    .await
    .map_err(super::map_join_error)??;

    let count = entries.len();
    api.upload_key_packages(entries).await?;
    Ok(count)
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

        let entries = tokio::task::spawn_blocking(move || {
            let mls = MlsManager::new(&data_dir, user_id)?;
            generate_initial_key_packages(&mls)
        })
        .await
        .map_err(super::map_join_error)??;

        api.upload_key_packages(entries).await?;
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
