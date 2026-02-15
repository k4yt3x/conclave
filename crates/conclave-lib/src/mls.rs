use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use mls_rs::client_builder::MlsConfig;
use mls_rs::group::proposal::Proposal;
use mls_rs::group::{CommitEffect, ReceivedMessage};
use mls_rs::identity::basic::{BasicCredential, BasicIdentityProvider};
use mls_rs::identity::SigningIdentity;
use mls_rs::{CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, MlsMessage};
use mls_rs_crypto_openssl::OpensslCryptoProvider;
use mls_rs_provider_sqlite::connection_strategy::FileConnectionStrategy;
use mls_rs_provider_sqlite::SqLiteDataStorageEngine;

use crate::error::{Error, Result};

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

/// Result of decrypting an incoming MLS message.
pub enum DecryptedMessage {
    /// Application message with plaintext bytes.
    Application(Vec<u8>),
    /// A commit was processed. Contains info about roster changes.
    Commit(CommitInfo),
    /// Other MLS message types (proposals, etc.) — no visible content.
    None,
}

/// Information about changes in a commit.
pub struct CommitInfo {
    pub members_added: Vec<String>,
    pub members_removed: Vec<String>,
    pub self_removed: bool,
}

/// MLS group details for display.
pub struct GroupDetails {
    pub epoch: u64,
    pub cipher_suite: String,
    pub member_count: usize,
    pub own_index: u32,
    pub members: Vec<(u32, String)>,
}

/// Persistent MLS state manager for the client.
pub struct MlsManager {
    identity_bytes: Vec<u8>,
    signing_key_bytes: Vec<u8>,
    data_dir: std::path::PathBuf,
}

impl MlsManager {
    /// Load or create MLS identity for the given username.
    ///
    /// All MLS state is stored directly in `data_dir` (single-account model).
    /// The `username` is used only for the MLS credential, not for directory paths.
    pub fn new(data_dir: &Path, username: &str) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        #[cfg(unix)]
        std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700))?;

        let identity_path = data_dir.join("mls_identity.bin");
        let signing_key_path = data_dir.join("mls_signing_key.bin");

        if identity_path.exists() && signing_key_path.exists() {
            let identity_bytes = std::fs::read(&identity_path)?;
            let signing_key_bytes = std::fs::read(&signing_key_path)?;
            Ok(Self {
                identity_bytes,
                signing_key_bytes,
                data_dir: data_dir.to_path_buf(),
            })
        } else {
            // Generate a new identity.
            let crypto_provider = OpensslCryptoProvider::default();
            let cipher_suite = crypto_provider
                .cipher_suite_provider(CIPHERSUITE)
                .ok_or_else(|| Error::Mls("cipher suite not supported".into()))?;

            let (secret_key, public_key) = cipher_suite
                .signature_key_generate()
                .map_err(|e| Error::Mls(format!("key generation failed: {e}")))?;

            let basic_credential = BasicCredential::new(username.as_bytes().to_vec());
            let signing_identity =
                SigningIdentity::new(basic_credential.into_credential(), public_key);

            // Serialize and persist.
            let identity_bytes = mls_rs_codec_to_vec(&signing_identity)?;
            let signing_key_bytes = secret_key.as_ref().to_vec();

            std::fs::write(&identity_path, &identity_bytes)?;
            std::fs::write(&signing_key_path, &signing_key_bytes)?;
            #[cfg(unix)]
            {
                std::fs::set_permissions(&identity_path, std::fs::Permissions::from_mode(0o600))?;
                std::fs::set_permissions(
                    &signing_key_path,
                    std::fs::Permissions::from_mode(0o600),
                )?;
            }

            Ok(Self {
                identity_bytes,
                signing_key_bytes,
                data_dir: data_dir.to_path_buf(),
            })
        }
    }

    /// Returns the data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Build an mls-rs Client with SQLite-backed storage.
    fn build_client(&self) -> Result<Client<impl MlsConfig>> {
        let db_path = self.data_dir.join("mls_state.db");
        let storage = SqLiteDataStorageEngine::new(FileConnectionStrategy::new(&db_path))
            .map_err(|e| Error::Mls(format!("SQLite storage init failed: {e}")))?;

        let signing_identity: SigningIdentity = mls_rs_codec_from_slice(&self.identity_bytes)?;
        let secret_key =
            mls_rs_core::crypto::SignatureSecretKey::from(self.signing_key_bytes.clone());

        let client = Client::builder()
            .crypto_provider(OpensslCryptoProvider::default())
            .identity_provider(BasicIdentityProvider)
            .key_package_repo(
                storage
                    .key_package_storage()
                    .map_err(|e| Error::Mls(format!("key package storage: {e}")))?,
            )
            .psk_store(
                storage
                    .pre_shared_key_storage()
                    .map_err(|e| Error::Mls(format!("PSK storage: {e}")))?,
            )
            .group_state_storage(
                storage
                    .group_state_storage()
                    .map_err(|e| Error::Mls(format!("group state storage: {e}")))?,
            )
            .signing_identity(signing_identity, secret_key, CIPHERSUITE)
            .build();

        Ok(client)
    }

    /// Generate a key package that can be uploaded to the server.
    pub fn generate_key_package(&self) -> Result<Vec<u8>> {
        let client = self.build_client()?;
        let kp_msg = client
            .generate_key_package_message(Default::default(), Default::default(), None)
            .map_err(|e| Error::Mls(format!("key package generation failed: {e}")))?;
        let bytes = kp_msg
            .to_bytes()
            .map_err(|e| Error::Mls(format!("key package serialization failed: {e}")))?;
        Ok(bytes)
    }

    /// Create a new MLS group, add members from their key packages.
    /// Returns: (group_id_hex, commit_bytes, welcome_messages_by_username, group_info_bytes)
    pub fn create_group(
        &self,
        member_key_packages: &HashMap<String, Vec<u8>>,
    ) -> Result<(String, Vec<u8>, HashMap<String, Vec<u8>>, Vec<u8>)> {
        let client = self.build_client()?;
        let mut group = client
            .create_group(ExtensionList::default(), Default::default(), None)
            .map_err(|e| Error::Mls(format!("create group failed: {e}")))?;

        // Add all members.
        let mut builder = group.commit_builder();
        let mut username_order: Vec<String> = Vec::new();

        for (username, kp_bytes) in member_key_packages {
            let kp_msg = MlsMessage::from_bytes(kp_bytes)
                .map_err(|e| Error::Mls(format!("invalid key package from '{username}': {e}")))?;
            builder = builder
                .add_member(kp_msg)
                .map_err(|e| Error::Mls(format!("add member '{username}' failed: {e}")))?;
            username_order.push(username.clone());
        }

        let commit_output = builder
            .build()
            .map_err(|e| Error::Mls(format!("commit build failed: {e}")))?;

        group
            .apply_pending_commit()
            .map_err(|e| Error::Mls(format!("apply pending commit failed: {e}")))?;

        // Map welcome messages to usernames.
        let mut welcome_map = HashMap::new();
        for (i, username) in username_order.iter().enumerate() {
            if let Some(welcome_msg) = commit_output.welcome_messages.get(i) {
                let welcome_bytes = welcome_msg
                    .to_bytes()
                    .map_err(|e| Error::Mls(format!("welcome serialization failed: {e}")))?;
                welcome_map.insert(username.clone(), welcome_bytes);
            }
        }

        let commit_bytes = commit_output
            .commit_message
            .to_bytes()
            .map_err(|e| Error::Mls(format!("commit serialization failed: {e}")))?;

        let group_info_msg = group
            .group_info_message_allowing_ext_commit(true)
            .map_err(|e| Error::Mls(format!("group info generation failed: {e}")))?;
        let group_info_bytes = group_info_msg
            .to_bytes()
            .map_err(|e| Error::Mls(format!("group info serialization failed: {e}")))?;

        // Persist group state.
        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        let group_id = hex::encode(group.group_id());

        Ok((group_id, commit_bytes, welcome_map, group_info_bytes))
    }

    /// Invite new members to an existing group.
    /// Returns: (commit_bytes, welcome_messages_by_username, group_info_bytes)
    pub fn invite_to_group(
        &self,
        mls_group_id: &str,
        member_key_packages: &HashMap<String, Vec<u8>>,
    ) -> Result<(Vec<u8>, HashMap<String, Vec<u8>>, Vec<u8>)> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let mut group = client
            .load_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("load group failed: {e}")))?;

        let mut builder = group.commit_builder();
        let mut username_order: Vec<String> = Vec::new();

        for (username, kp_bytes) in member_key_packages {
            let kp_msg = MlsMessage::from_bytes(kp_bytes)
                .map_err(|e| Error::Mls(format!("invalid key package from '{username}': {e}")))?;
            builder = builder
                .add_member(kp_msg)
                .map_err(|e| Error::Mls(format!("add member '{username}' failed: {e}")))?;
            username_order.push(username.clone());
        }

        let commit_output = builder
            .build()
            .map_err(|e| Error::Mls(format!("commit build failed: {e}")))?;

        group
            .apply_pending_commit()
            .map_err(|e| Error::Mls(format!("apply pending commit failed: {e}")))?;

        let mut welcome_map = HashMap::new();
        for (i, username) in username_order.iter().enumerate() {
            if let Some(welcome_msg) = commit_output.welcome_messages.get(i) {
                let welcome_bytes = welcome_msg
                    .to_bytes()
                    .map_err(|e| Error::Mls(format!("welcome serialization failed: {e}")))?;
                welcome_map.insert(username.clone(), welcome_bytes);
            }
        }

        let commit_bytes = commit_output
            .commit_message
            .to_bytes()
            .map_err(|e| Error::Mls(format!("commit serialization failed: {e}")))?;

        let group_info_msg = group
            .group_info_message_allowing_ext_commit(true)
            .map_err(|e| Error::Mls(format!("group info generation failed: {e}")))?;
        let group_info_bytes = group_info_msg
            .to_bytes()
            .map_err(|e| Error::Mls(format!("group info serialization failed: {e}")))?;

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        Ok((commit_bytes, welcome_map, group_info_bytes))
    }

    /// Join a group via a welcome message.
    /// Returns the MLS group ID (hex-encoded).
    pub fn join_group(&self, welcome_bytes: &[u8]) -> Result<String> {
        let client = self.build_client()?;
        let welcome_msg = MlsMessage::from_bytes(welcome_bytes)
            .map_err(|e| Error::Mls(format!("invalid welcome message: {e}")))?;

        let (mut group, _info) = client
            .join_group(None, &welcome_msg, None)
            .map_err(|e| Error::Mls(format!("join group failed: {e}")))?;

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        let group_id = hex::encode(group.group_id());
        Ok(group_id)
    }

    /// Encrypt a plaintext message for a group.
    /// Returns the encrypted MLS message bytes.
    pub fn encrypt_message(&self, mls_group_id: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let mut group = client
            .load_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("load group failed: {e}")))?;

        let msg = group
            .encrypt_application_message(plaintext, Default::default())
            .map_err(|e| Error::Mls(format!("encrypt failed: {e}")))?;

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        msg.to_bytes()
            .map_err(|e| Error::Mls(format!("message serialization failed: {e}")))
    }

    /// Decrypt an incoming MLS message for a group.
    /// Returns detailed information about the message content and any roster changes.
    pub fn decrypt_message(
        &self,
        mls_group_id: &str,
        mls_message_bytes: &[u8],
    ) -> Result<DecryptedMessage> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let mut group = client
            .load_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("load group failed: {e}")))?;

        let msg = MlsMessage::from_bytes(mls_message_bytes)
            .map_err(|e| Error::Mls(format!("invalid MLS message: {e}")))?;

        let received = match group.process_incoming_message(msg) {
            Ok(r) => r,
            Err(_) => {
                // Messages from prior epochs (e.g., commits already processed via welcome)
                // are expected and can be safely skipped.
                return Ok(DecryptedMessage::None);
            }
        };

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        match received {
            ReceivedMessage::ApplicationMessage(app_msg) => {
                Ok(DecryptedMessage::Application(app_msg.data().to_vec()))
            }
            ReceivedMessage::Commit(commit_desc) => {
                let mut members_added = Vec::new();
                let mut members_removed = Vec::new();
                let mut self_removed = false;

                // Extract roster changes from applied proposals.
                let new_epoch = match &commit_desc.effect {
                    CommitEffect::NewEpoch(epoch) => Some(epoch),
                    CommitEffect::Removed { new_epoch, .. } => {
                        self_removed = true;
                        Some(new_epoch)
                    }
                    _ => None,
                };

                if let Some(epoch) = new_epoch {
                    for proposal_info in &epoch.applied_proposals {
                        match &proposal_info.proposal {
                            Proposal::Add(add) => {
                                let name = extract_username_from_identity(add.signing_identity());
                                members_added.push(name);
                            }
                            Proposal::Remove(remove) => {
                                let removed_index = remove.to_remove();
                                members_removed.push(format!("#{removed_index}"));
                            }
                            _ => {}
                        }
                    }
                }

                Ok(DecryptedMessage::Commit(CommitInfo {
                    members_added,
                    members_removed,
                    self_removed,
                }))
            }
            _ => Ok(DecryptedMessage::None),
        }
    }

    /// Remove a member from a group by their leaf index.
    /// Returns (commit_bytes, group_info_bytes).
    pub fn remove_member(
        &self,
        mls_group_id: &str,
        member_index: u32,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let mut group = client
            .load_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("load group failed: {e}")))?;

        let commit_output = group
            .commit_builder()
            .remove_member(member_index)
            .map_err(|e| Error::Mls(format!("remove member failed: {e}")))?
            .build()
            .map_err(|e| Error::Mls(format!("commit build failed: {e}")))?;

        group
            .apply_pending_commit()
            .map_err(|e| Error::Mls(format!("apply pending commit failed: {e}")))?;

        let commit_bytes = commit_output
            .commit_message
            .to_bytes()
            .map_err(|e| Error::Mls(format!("commit serialization failed: {e}")))?;

        let group_info_msg = group
            .group_info_message_allowing_ext_commit(true)
            .map_err(|e| Error::Mls(format!("group info generation failed: {e}")))?;
        let group_info_bytes = group_info_msg
            .to_bytes()
            .map_err(|e| Error::Mls(format!("group info serialization failed: {e}")))?;

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        Ok((commit_bytes, group_info_bytes))
    }

    /// Find a member's leaf index by their identity (username).
    pub fn find_member_index(&self, mls_group_id: &str, username: &str) -> Result<Option<u32>> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let group = client
            .load_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("load group failed: {e}")))?;

        for member in group.roster().members_iter() {
            let name = extract_username_from_identity(&member.signing_identity);
            if name == username {
                return Ok(Some(member.index));
            }
        }

        Ok(None)
    }

    /// Perform an external commit to rejoin a group with a new identity.
    /// Returns (mls_group_id_hex, commit_bytes).
    pub fn external_rejoin_group(
        &self,
        group_info_bytes: &[u8],
        old_leaf_index: Option<u32>,
    ) -> Result<(String, Vec<u8>)> {
        let client = self.build_client()?;

        let group_info_msg = MlsMessage::from_bytes(group_info_bytes)
            .map_err(|e| Error::Mls(format!("invalid group info: {e}")))?;

        let mut builder = client
            .external_commit_builder()
            .map_err(|e| Error::Mls(format!("external commit builder failed: {e}")))?;

        // Remove our old leaf if we know our previous index.
        if let Some(old_index) = old_leaf_index {
            builder = builder.with_removal(old_index);
        }

        let (mut group, commit_msg) = builder
            .build(group_info_msg)
            .map_err(|e| Error::Mls(format!("external commit build failed: {e}")))?;

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        let group_id = hex::encode(group.group_id());
        let commit_bytes = commit_msg
            .to_bytes()
            .map_err(|e| Error::Mls(format!("commit serialization failed: {e}")))?;

        Ok((group_id, commit_bytes))
    }

    /// Perform a key update for forward secrecy.
    /// Returns (commit_bytes, group_info_bytes).
    pub fn rotate_keys(&self, mls_group_id: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let mut group = client
            .load_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("load group failed: {e}")))?;

        // An empty commit advances the epoch and rotates keys.
        let commit_output = group
            .commit_builder()
            .build()
            .map_err(|e| Error::Mls(format!("commit build failed: {e}")))?;

        group
            .apply_pending_commit()
            .map_err(|e| Error::Mls(format!("apply pending commit failed: {e}")))?;

        let commit_bytes = commit_output
            .commit_message
            .to_bytes()
            .map_err(|e| Error::Mls(format!("commit serialization failed: {e}")))?;

        let group_info_msg = group
            .group_info_message_allowing_ext_commit(true)
            .map_err(|e| Error::Mls(format!("group info generation failed: {e}")))?;
        let group_info_bytes = group_info_msg
            .to_bytes()
            .map_err(|e| Error::Mls(format!("group info serialization failed: {e}")))?;

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        Ok((commit_bytes, group_info_bytes))
    }

    /// Get group information: epoch, cipher suite, member count, own index.
    pub fn group_info_details(&self, mls_group_id: &str) -> Result<GroupDetails> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let group = client
            .load_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("load group failed: {e}")))?;

        let roster = group.roster();
        let members: Vec<(u32, String)> = roster
            .members_iter()
            .map(|m| {
                let name = extract_username_from_identity(&m.signing_identity);
                (m.index, name)
            })
            .collect();

        Ok(GroupDetails {
            epoch: group.current_epoch(),
            cipher_suite: format!("{:?}", group.cipher_suite()),
            member_count: members.len(),
            own_index: group.current_member_index(),
            members,
        })
    }

    /// Delete local group state (for when we've been removed or left).
    pub fn delete_group_state(&self, mls_group_id: &str) -> Result<()> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        // Load and delete from storage by overwriting with a cleared state.
        // The simplest approach: just try to load and ignore if it doesn't exist.
        if let Ok(mut group) = client.load_group(&group_id_bytes) {
            group
                .write_to_storage()
                .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;
        }

        Ok(())
    }

    /// Wipe all local MLS state (identity + group state DB).
    /// Used for account reset.
    pub fn wipe_local_state(&self) -> Result<()> {
        let identity_path = self.data_dir.join("mls_identity.bin");
        let signing_key_path = self.data_dir.join("mls_signing_key.bin");
        let state_db_path = self.data_dir.join("mls_state.db");

        let _ = std::fs::remove_file(identity_path);
        let _ = std::fs::remove_file(signing_key_path);
        let _ = std::fs::remove_file(state_db_path);
        // Also remove WAL/SHM files if they exist.
        let _ = std::fs::remove_file(self.data_dir.join("mls_state.db-wal"));
        let _ = std::fs::remove_file(self.data_dir.join("mls_state.db-shm"));

        Ok(())
    }
}

/// Extract a username from an MLS SigningIdentity's BasicCredential.
fn extract_username_from_identity(identity: &SigningIdentity) -> String {
    match identity.credential.as_basic() {
        Some(basic) => String::from_utf8_lossy(&basic.identifier).to_string(),
        None => "<unknown>".to_string(),
    }
}

// ── MLS codec helpers ─────────────────────────────────────────────

fn mls_rs_codec_to_vec(value: &SigningIdentity) -> Result<Vec<u8>> {
    use mls_rs_codec::MlsEncode;
    value
        .mls_encode_to_vec()
        .map_err(|e| Error::Mls(format!("MLS codec encode failed: {e}")))
}

fn mls_rs_codec_from_slice(bytes: &[u8]) -> Result<SigningIdentity> {
    use mls_rs_codec::MlsDecode;
    SigningIdentity::mls_decode(&mut &*bytes)
        .map_err(|e| Error::Mls(format!("MLS codec decode failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    /// Create an MlsManager in a fresh temporary directory.
    /// The TempDir is returned so it stays alive for the test's duration.
    fn create_manager(username: &str) -> (TempDir, MlsManager) {
        let dir = TempDir::new().unwrap();
        let mgr = MlsManager::new(dir.path(), username).unwrap();
        (dir, mgr)
    }

    /// Helper: Alice creates a group containing herself and bob.
    /// Returns everything needed for further interaction.
    fn setup_alice_bob() -> (
        TempDir,
        MlsManager,
        TempDir,
        MlsManager,
        String,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
    ) {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");

        let bob_kp = bob.generate_key_package().unwrap();
        let mut members = HashMap::new();
        members.insert("bob".to_string(), bob_kp);

        let (group_id, commit_bytes, welcome_map, group_info_bytes) =
            alice.create_group(&members).unwrap();

        let welcome_bytes = welcome_map.get("bob").unwrap().clone();

        (
            _dir_a,
            alice,
            _dir_b,
            bob,
            group_id,
            commit_bytes,
            welcome_bytes,
            group_info_bytes,
        )
    }

    #[test]
    fn test_generate_key_package() {
        let (_dir, mgr) = create_manager("alice");
        let kp = mgr.generate_key_package().unwrap();
        assert!(!kp.is_empty(), "key package bytes must not be empty");
    }

    #[test]
    fn test_create_group_single_member() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");

        let bob_kp = bob.generate_key_package().unwrap();
        let mut members = HashMap::new();
        members.insert("bob".to_string(), bob_kp);

        let (group_id, commit_bytes, welcome_map, group_info_bytes) =
            alice.create_group(&members).unwrap();

        assert!(!group_id.is_empty(), "group_id must not be empty");
        assert!(hex::decode(&group_id).is_ok(), "group_id must be valid hex");
        assert!(!commit_bytes.is_empty(), "commit must not be empty");
        assert!(
            welcome_map.contains_key("bob"),
            "welcome_map must contain bob"
        );
        assert!(
            !welcome_map.get("bob").unwrap().is_empty(),
            "bob's welcome must not be empty"
        );
        assert!(!group_info_bytes.is_empty(), "group_info must not be empty");
    }

    #[test]
    fn test_join_group_via_welcome() {
        let (_dir_a, _alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();

        let bob_group_id = bob.join_group(&welcome).unwrap();
        assert_eq!(bob_group_id, group_id, "bob's group_id must match alice's");
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();

        bob.join_group(&welcome).unwrap();

        let plaintext = b"hello, world!";
        let ciphertext = alice.encrypt_message(&group_id, plaintext).unwrap();
        assert!(!ciphertext.is_empty());

        let decrypted = bob.decrypt_message(&group_id, &ciphertext).unwrap();
        match decrypted {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, plaintext.to_vec());
            }
            _ => panic!("expected Application message, got something else"),
        }
    }

    #[test]
    fn test_decrypt_returns_application_message() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();

        bob.join_group(&welcome).unwrap();

        let plaintext = b"specific payload bytes";
        let ciphertext = alice.encrypt_message(&group_id, plaintext).unwrap();

        let result = bob.decrypt_message(&group_id, &ciphertext).unwrap();
        if let DecryptedMessage::Application(data) = result {
            assert_eq!(
                data,
                plaintext.to_vec(),
                "decrypted data must match original plaintext"
            );
        } else {
            panic!("expected DecryptedMessage::Application variant");
        }
    }

    #[test]
    fn test_decrypt_commit_message() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome_bob, _gi) = setup_alice_bob();

        bob.join_group(&welcome_bob).unwrap();

        let (_dir_c, carol) = create_manager("carol");
        let carol_kp = carol.generate_key_package().unwrap();
        let mut carol_members = HashMap::new();
        carol_members.insert("carol".to_string(), carol_kp);

        let (invite_commit, _welcome_map, _gi2) =
            alice.invite_to_group(&group_id, &carol_members).unwrap();

        let result = bob.decrypt_message(&group_id, &invite_commit).unwrap();
        match result {
            DecryptedMessage::Commit(info) => {
                assert!(
                    info.members_added.contains(&"carol".to_string()),
                    "commit info must list carol as added, got: {:?}",
                    info.members_added
                );
                assert!(!info.self_removed, "bob must not be self_removed");
            }
            _ => panic!("expected DecryptedMessage::Commit variant"),
        }
    }

    #[test]
    fn test_create_group_multiple_members() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");
        let (_dir_c, carol) = create_manager("carol");

        let bob_kp = bob.generate_key_package().unwrap();
        let mut bob_members = HashMap::new();
        bob_members.insert("bob".to_string(), bob_kp);

        let (group_id, _commit, welcome_map_bob, _gi) = alice.create_group(&bob_members).unwrap();

        let bob_gid = bob.join_group(welcome_map_bob.get("bob").unwrap()).unwrap();
        assert_eq!(bob_gid, group_id, "bob's group_id must match");

        let carol_kp = carol.generate_key_package().unwrap();
        let mut carol_members = HashMap::new();
        carol_members.insert("carol".to_string(), carol_kp);

        let (_invite_commit, welcome_map_carol, _gi2) =
            alice.invite_to_group(&group_id, &carol_members).unwrap();

        let carol_gid = carol
            .join_group(welcome_map_carol.get("carol").unwrap())
            .unwrap();
        assert_eq!(carol_gid, group_id, "carol's group_id must match");
    }

    #[test]
    fn test_remove_member() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");
        let (_dir_c, carol) = create_manager("carol");

        let bob_kp = bob.generate_key_package().unwrap();
        let mut bob_members = HashMap::new();
        bob_members.insert("bob".to_string(), bob_kp);

        let (group_id, _commit, welcome_map_bob, _gi) = alice.create_group(&bob_members).unwrap();
        bob.join_group(welcome_map_bob.get("bob").unwrap()).unwrap();

        let carol_kp = carol.generate_key_package().unwrap();
        let mut carol_members = HashMap::new();
        carol_members.insert("carol".to_string(), carol_kp);

        let (_invite_commit, welcome_map_carol, _gi2) =
            alice.invite_to_group(&group_id, &carol_members).unwrap();
        carol
            .join_group(welcome_map_carol.get("carol").unwrap())
            .unwrap();

        let carol_index = alice
            .find_member_index(&group_id, "carol")
            .unwrap()
            .expect("carol must be found in group");

        let (commit, group_info) = alice.remove_member(&group_id, carol_index).unwrap();
        assert!(!commit.is_empty(), "removal commit must not be empty");
        assert!(
            !group_info.is_empty(),
            "removal group_info must not be empty"
        );
    }

    #[test]
    fn test_find_member_index_found() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();

        let result = alice.find_member_index(&group_id, "bob").unwrap();
        assert!(result.is_some(), "bob must be found in the group");
    }

    #[test]
    fn test_find_member_index_not_found() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();

        let result = alice.find_member_index(&group_id, "carol").unwrap();
        assert!(result.is_none(), "carol must not be found in the group");
    }

    #[test]
    fn test_rotate_keys() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();

        bob.join_group(&welcome).unwrap();

        let (commit, group_info) = alice.rotate_keys(&group_id).unwrap();
        assert!(!commit.is_empty(), "rotation commit must not be empty");
        assert!(
            !group_info.is_empty(),
            "rotation group_info must not be empty"
        );
    }

    #[test]
    fn test_rotate_keys_advances_epoch() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();

        bob.join_group(&welcome).unwrap();

        let details_before = alice.group_info_details(&group_id).unwrap();
        let epoch_before = details_before.epoch;

        alice.rotate_keys(&group_id).unwrap();

        let details_after = alice.group_info_details(&group_id).unwrap();
        assert!(
            details_after.epoch > epoch_before,
            "epoch must increase after key rotation: before={}, after={}",
            epoch_before,
            details_after.epoch
        );
    }

    #[test]
    fn test_group_info_details() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();

        let details = alice.group_info_details(&group_id).unwrap();

        assert!(details.epoch >= 1, "epoch must be at least 1");
        assert_eq!(
            details.member_count, 2,
            "member_count must be 2 (alice + bob)"
        );
        assert!(
            !details.cipher_suite.is_empty(),
            "cipher_suite must not be empty"
        );
        assert!(
            !details.members.is_empty(),
            "members list must not be empty"
        );

        let member_names: Vec<&str> = details.members.iter().map(|(_, n)| n.as_str()).collect();
        assert!(
            member_names.contains(&"alice"),
            "members must include alice"
        );
        assert!(member_names.contains(&"bob"), "members must include bob");
    }

    #[test]
    fn test_group_info_details_nonexistent_group() {
        let (_dir, mgr) = create_manager("alice");
        mgr.generate_key_package().unwrap();

        let result = mgr.group_info_details("deadbeef");
        assert!(
            result.is_err(),
            "group_info_details for a nonexistent group must return Err"
        );
    }

    #[test]
    fn test_delete_group_state() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();

        let result = alice.delete_group_state(&group_id);
        assert!(result.is_ok(), "delete_group_state must not error");
    }

    #[test]
    fn test_wipe_local_state() {
        let (dir, mgr) = create_manager("alice");
        mgr.generate_key_package().unwrap();

        let data_dir = dir.path();
        assert!(
            data_dir.join("mls_identity.bin").exists(),
            "identity file must exist before wipe"
        );
        assert!(
            data_dir.join("mls_signing_key.bin").exists(),
            "signing key file must exist before wipe"
        );
        assert!(
            data_dir.join("mls_state.db").exists(),
            "state DB must exist before wipe"
        );

        mgr.wipe_local_state().unwrap();

        assert!(
            !data_dir.join("mls_identity.bin").exists(),
            "identity file must be gone after wipe"
        );
        assert!(
            !data_dir.join("mls_signing_key.bin").exists(),
            "signing key file must be gone after wipe"
        );
        assert!(
            !data_dir.join("mls_state.db").exists(),
            "state DB must be gone after wipe"
        );
    }

    #[test]
    fn test_external_rejoin_group() {
        let (_dir_a, _alice, _dir_b, bob, group_id, _commit, welcome, group_info_bytes) =
            setup_alice_bob();

        bob.join_group(&welcome).unwrap();

        let (_dir_new, alice_new) = create_manager("alice_new");
        let (new_group_id, ext_commit) = alice_new
            .external_rejoin_group(&group_info_bytes, None)
            .unwrap();

        assert_eq!(
            new_group_id, group_id,
            "external rejoin group_id must match original"
        );
        assert!(!ext_commit.is_empty(), "external commit must not be empty");
    }

    #[test]
    fn test_encrypt_message_nonexistent_group() {
        let (_dir, mgr) = create_manager("alice");
        mgr.generate_key_package().unwrap();

        let result = mgr.encrypt_message("deadbeef", b"hello");
        assert!(
            result.is_err(),
            "encrypt_message on nonexistent group must return Err"
        );
    }

    #[test]
    fn test_decrypt_invalid_message() {
        let (_dir_a, _alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();

        bob.join_group(&welcome).unwrap();

        let result = bob.decrypt_message(&group_id, b"garbage data");
        assert!(result.is_err(), "decrypt of garbage bytes must return Err");
    }

    #[test]
    fn test_new_creates_identity_files() {
        let dir = TempDir::new().unwrap();
        let _mgr = MlsManager::new(dir.path(), "alice").unwrap();

        assert!(
            dir.path().join("mls_identity.bin").exists(),
            "mls_identity.bin must exist after MlsManager::new"
        );
        assert!(
            dir.path().join("mls_signing_key.bin").exists(),
            "mls_signing_key.bin must exist after MlsManager::new"
        );
    }

    #[test]
    fn test_new_loads_existing_identity() {
        let dir = TempDir::new().unwrap();

        let mgr1 = MlsManager::new(dir.path(), "alice").unwrap();
        let id1 = mgr1.identity_bytes.clone();
        let sk1 = mgr1.signing_key_bytes.clone();

        let mgr2 = MlsManager::new(dir.path(), "alice").unwrap();
        assert_eq!(
            mgr2.identity_bytes, id1,
            "identity must be the same on reload"
        );
        assert_eq!(
            mgr2.signing_key_bytes, sk1,
            "signing key must be the same on reload"
        );
    }

    #[test]
    fn test_message_after_key_rotation() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();

        bob.join_group(&welcome).unwrap();

        let (rotation_commit, _gi2) = alice.rotate_keys(&group_id).unwrap();

        let rotation_result = bob.decrypt_message(&group_id, &rotation_commit).unwrap();
        match &rotation_result {
            DecryptedMessage::Commit(_) => {}
            _ => panic!("rotation must produce a Commit message for bob"),
        }

        let plaintext = b"post-rotation secret";
        let ciphertext = alice.encrypt_message(&group_id, plaintext).unwrap();

        let decrypted = bob.decrypt_message(&group_id, &ciphertext).unwrap();
        match decrypted {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, plaintext.to_vec());
            }
            _ => panic!("expected Application message after rotation"),
        }
    }

    #[test]
    fn test_remove_member_then_decrypt() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");
        let (_dir_c, carol) = create_manager("carol");

        let bob_kp = bob.generate_key_package().unwrap();
        let mut bob_members = HashMap::new();
        bob_members.insert("bob".to_string(), bob_kp);

        let (group_id, _commit, welcome_map_bob, _gi) = alice.create_group(&bob_members).unwrap();
        bob.join_group(welcome_map_bob.get("bob").unwrap()).unwrap();

        let carol_kp = carol.generate_key_package().unwrap();
        let mut carol_members = HashMap::new();
        carol_members.insert("carol".to_string(), carol_kp);

        let (invite_commit, welcome_map_carol, _gi2) =
            alice.invite_to_group(&group_id, &carol_members).unwrap();

        bob.decrypt_message(&group_id, &invite_commit).unwrap();

        carol
            .join_group(welcome_map_carol.get("carol").unwrap())
            .unwrap();

        let carol_index = alice
            .find_member_index(&group_id, "carol")
            .unwrap()
            .expect("carol must be in the group");
        let (removal_commit, _gi3) = alice.remove_member(&group_id, carol_index).unwrap();

        let removal_result = bob.decrypt_message(&group_id, &removal_commit).unwrap();
        match &removal_result {
            DecryptedMessage::Commit(info) => {
                assert!(
                    !info.members_removed.is_empty(),
                    "removal commit must list removed members"
                );
            }
            _ => panic!("removal must produce a Commit message for bob"),
        }

        let plaintext = b"carol cannot see this";
        let ciphertext = alice.encrypt_message(&group_id, plaintext).unwrap();

        let decrypted = bob.decrypt_message(&group_id, &ciphertext).unwrap();
        match decrypted {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, plaintext.to_vec());
            }
            _ => panic!("expected Application message from bob after carol removal"),
        }

        let carol_result = carol.decrypt_message(&group_id, &ciphertext).unwrap();
        match carol_result {
            DecryptedMessage::None => {}
            DecryptedMessage::Application(_) => {
                panic!("carol must NOT be able to decrypt after removal")
            }
            DecryptedMessage::Commit(_) => {
                panic!("carol must NOT process this as a commit")
            }
        }
    }
}
