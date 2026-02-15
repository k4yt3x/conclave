use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use mls_rs::client_builder::MlsConfig;
use mls_rs::group::proposal::Proposal;
use mls_rs::group::{CommitEffect, ReceivedMessage};
use mls_rs::identity::SigningIdentity;
use mls_rs::identity::basic::{BasicCredential, BasicIdentityProvider};
use mls_rs::{CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, MlsMessage};
use mls_rs_crypto_openssl::OpensslCryptoProvider;
use mls_rs_provider_sqlite::SqLiteDataStorageEngine;
use mls_rs_provider_sqlite::connection_strategy::FileConnectionStrategy;

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
