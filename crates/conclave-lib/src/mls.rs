use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use mls_rs::client_builder::MlsConfig;
use mls_rs::extension::recommended::LastResortKeyPackageExt;
use mls_rs::group::proposal::Proposal;
use mls_rs::group::{CommitEffect, ReceivedMessage};
use mls_rs::identity::SigningIdentity;
use mls_rs::identity::basic::{BasicCredential, BasicIdentityProvider};
use mls_rs::error::MlsError;
use mls_rs::{
    CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, KeyPackageRef,
    MlsMessage,
};
use mls_rs_crypto_openssl::OpensslCryptoProvider;
use mls_rs_provider_sqlite::SqLiteDataStorageEngine;
use mls_rs_provider_sqlite::connection_strategy::FileConnectionStrategy;

use crate::error::{Error, Result};

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE448_CHACHA;

/// Number of prior MLS epochs to retain for decrypting old messages.
///
/// The mls-rs default is 3, which is too tight — if a client is offline while
/// 4+ commits occur (invites, kicks, key rotations), messages from the oldest
/// epoch become permanently undecryptable. A value of 16 gives a comfortable
/// buffer for typical offline periods at minimal storage cost.
const EPOCH_RETENTION: u64 = 16;

/// Result of decrypting an incoming MLS message.
#[derive(Debug)]
pub enum DecryptedMessage {
    /// Application message with plaintext bytes.
    Application(Vec<u8>),
    /// A commit was processed. Contains info about roster changes.
    Commit(CommitInfo),
    /// Other MLS message types (proposals, etc.) — no visible content.
    None,
    /// Decryption failed with a meaningful error (e.g., epoch evicted, key
    /// missing). Callers should notify the user and may suggest `/reset`.
    Failed(String),
}

/// Information about changes in a commit.
#[derive(Debug)]
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

        let group_state = storage
            .group_state_storage()
            .map_err(|e| Error::Mls(format!("group state storage: {e}")))?
            .with_max_epoch_retention(EPOCH_RETENTION);

        let client = Client::builder()
            .crypto_provider(OpensslCryptoProvider::default())
            .identity_provider(BasicIdentityProvider)
            .key_package_lifetime(Duration::from_secs(90 * 24 * 3600)) // 90 days
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
            .group_state_storage(group_state)
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

    /// Generate a last-resort key package (RFC 9420 §16.8 best practice).
    ///
    /// A last-resort key package carries the `LastResortKeyPackageExt` extension,
    /// signalling that it should not be deleted from the server after consumption.
    pub fn generate_last_resort_key_package(&self) -> Result<Vec<u8>> {
        let client = self.build_client()?;
        let mut exts = ExtensionList::new();
        exts.set_from(LastResortKeyPackageExt)
            .map_err(|e| Error::Mls(format!("set extension failed: {e}")))?;

        let kp_msg = client
            .generate_key_package_message(exts, Default::default(), None)
            .map_err(|e| Error::Mls(format!("last resort key package generation failed: {e}")))?;
        let bytes = kp_msg
            .to_bytes()
            .map_err(|e| Error::Mls(format!("key package serialization failed: {e}")))?;
        Ok(bytes)
    }

    /// Generate N regular key packages for batch upload.
    pub fn generate_key_packages(&self, count: usize) -> Result<Vec<Vec<u8>>> {
        let client = self.build_client()?;
        let mut packages = Vec::with_capacity(count);
        for _ in 0..count {
            let kp_msg = client
                .generate_key_package_message(Default::default(), Default::default(), None)
                .map_err(|e| Error::Mls(format!("key package generation failed: {e}")))?;
            let bytes = kp_msg
                .to_bytes()
                .map_err(|e| Error::Mls(format!("key package serialization failed: {e}")))?;
            packages.push(bytes);
        }
        Ok(packages)
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

        // Add all members, tracking each key package reference for
        // welcome matching (RFC 9420 §12.4.3).
        let cipher_suite = OpensslCryptoProvider::default()
            .cipher_suite_provider(CIPHERSUITE)
            .ok_or_else(|| Error::Mls("cipher suite not supported".into()))?;

        let mut builder = group.commit_builder();
        let mut kp_ref_to_username: HashMap<KeyPackageRef, String> = HashMap::new();

        for (username, kp_bytes) in member_key_packages {
            let kp_msg = MlsMessage::from_bytes(kp_bytes)
                .map_err(|e| Error::Mls(format!("invalid key package from '{username}': {e}")))?;
            if let Some(kp_ref) = kp_msg
                .key_package_reference(&cipher_suite)
                .map_err(|e| Error::Mls(format!("key package ref for '{username}': {e}")))?
            {
                kp_ref_to_username.insert(kp_ref, username.clone());
            }
            builder = builder
                .add_member(kp_msg)
                .map_err(|e| Error::Mls(format!("add member '{username}' failed: {e}")))?;
        }

        let commit_output = builder
            .build()
            .map_err(|e| Error::Mls(format!("commit build failed: {e}")))?;

        group
            .apply_pending_commit()
            .map_err(|e| Error::Mls(format!("apply pending commit failed: {e}")))?;

        // Match each welcome message to its recipient by KeyPackage
        // reference rather than relying on array index ordering.
        let mut welcome_map = HashMap::new();
        for welcome_msg in &commit_output.welcome_messages {
            let welcome_bytes = welcome_msg
                .to_bytes()
                .map_err(|e| Error::Mls(format!("welcome serialization failed: {e}")))?;
            for kp_ref in welcome_msg.welcome_key_package_references() {
                if let Some(username) = kp_ref_to_username.get(kp_ref) {
                    welcome_map.insert(username.clone(), welcome_bytes.clone());
                }
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

        let cipher_suite = OpensslCryptoProvider::default()
            .cipher_suite_provider(CIPHERSUITE)
            .ok_or_else(|| Error::Mls("cipher suite not supported".into()))?;

        let mut builder = group.commit_builder();
        let mut kp_ref_to_username: HashMap<KeyPackageRef, String> = HashMap::new();

        for (username, kp_bytes) in member_key_packages {
            let kp_msg = MlsMessage::from_bytes(kp_bytes)
                .map_err(|e| Error::Mls(format!("invalid key package from '{username}': {e}")))?;
            if let Some(kp_ref) = kp_msg
                .key_package_reference(&cipher_suite)
                .map_err(|e| Error::Mls(format!("key package ref for '{username}': {e}")))?
            {
                kp_ref_to_username.insert(kp_ref, username.clone());
            }
            builder = builder
                .add_member(kp_msg)
                .map_err(|e| Error::Mls(format!("add member '{username}' failed: {e}")))?;
        }

        let commit_output = builder
            .build()
            .map_err(|e| Error::Mls(format!("commit build failed: {e}")))?;

        group
            .apply_pending_commit()
            .map_err(|e| Error::Mls(format!("apply pending commit failed: {e}")))?;

        let mut welcome_map = HashMap::new();
        for welcome_msg in &commit_output.welcome_messages {
            let welcome_bytes = welcome_msg
                .to_bytes()
                .map_err(|e| Error::Mls(format!("welcome serialization failed: {e}")))?;
            for kp_ref in welcome_msg.welcome_key_package_references() {
                if let Some(username) = kp_ref_to_username.get(kp_ref) {
                    welcome_map.insert(username.clone(), welcome_bytes.clone());
                }
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
            Err(e) => match e {
                // Our own commits (e.g., the initial commit already applied
                // via welcome) — harmless, silently skip.
                MlsError::CantProcessMessageFromSelf => return Ok(DecryptedMessage::None),
                // Messages from epochs before the client joined (e.g., the
                // group-creation commit when we joined via welcome) cannot
                // be decrypted because the key material was never available.
                MlsError::InvalidEpoch => return Ok(DecryptedMessage::None),
                // All other errors indicate a real problem: key missing,
                // invalid signature, state desync, etc.
                _ => return Ok(DecryptedMessage::Failed(e.to_string())),
            },
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

    /// Leave a group by producing a Remove commit for our own member index.
    ///
    /// Returns `Ok(Some((commit_bytes, group_info_bytes)))` if the self-remove
    /// commit succeeds, or `Ok(None)` if mls-rs does not support committing
    /// our own removal (in which case a remaining member must remove us).
    pub fn leave_group(&self, mls_group_id: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let client = self.build_client()?;
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let mut group = client
            .load_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("load group failed: {e}")))?;

        let own_index = group.current_member_index();

        let commit_output = match group
            .commit_builder()
            .remove_member(own_index)
            .and_then(|builder| builder.build())
        {
            Ok(output) => output,
            Err(_) => return Ok(None),
        };

        if group.apply_pending_commit().is_err() {
            return Ok(None);
        }

        let commit_bytes = commit_output
            .commit_message
            .to_bytes()
            .map_err(|e| Error::Mls(format!("commit serialization failed: {e}")))?;

        // Generate group info before we delete our state. This may fail if the
        // committer has been removed from the resulting state.
        let group_info_bytes = match group.group_info_message_allowing_ext_commit(true) {
            Ok(gi) => gi
                .to_bytes()
                .map_err(|e| Error::Mls(format!("group info serialization failed: {e}")))?,
            Err(_) => Vec::new(),
        };

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        Ok(Some((commit_bytes, group_info_bytes)))
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
        let group_id_bytes =
            hex::decode(mls_group_id).map_err(|e| Error::Mls(format!("invalid group ID: {e}")))?;

        let db_path = self.data_dir.join("mls_state.db");
        let storage = SqLiteDataStorageEngine::new(FileConnectionStrategy::new(&db_path))
            .map_err(|e| Error::Mls(format!("SQLite storage init failed: {e}")))?;

        let group_state = storage
            .group_state_storage()
            .map_err(|e| Error::Mls(format!("group state storage: {e}")))?;

        group_state
            .delete_group(&group_id_bytes)
            .map_err(|e| Error::Mls(format!("delete group state failed: {e}")))?;

        Ok(())
    }

    /// Wipe all local MLS state (identity + group state DB).
    /// Used for account reset.
    pub fn wipe_local_state(&self) -> Result<()> {
        let identity_path = self.data_dir.join("mls_identity.bin");
        let signing_key_path = self.data_dir.join("mls_signing_key.bin");
        let state_db_path = self.data_dir.join("mls_state.db");

        for path in [
            identity_path,
            signing_key_path,
            state_db_path,
            self.data_dir.join("mls_state.db-wal"),
            self.data_dir.join("mls_state.db-shm"),
        ] {
            if let Err(error) = std::fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(?path, %error, "failed to remove MLS state file");
                }
            }
        }

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
    fn test_generate_last_resort_key_package() {
        let (_dir, mgr) = create_manager("alice");
        let kp = mgr.generate_last_resort_key_package().unwrap();
        assert!(
            !kp.is_empty(),
            "last resort key package bytes must not be empty"
        );
    }

    #[test]
    fn test_generate_key_packages_batch() {
        let (_dir, mgr) = create_manager("alice");
        let packages = mgr.generate_key_packages(5).unwrap();
        assert_eq!(packages.len(), 5);
        for kp in &packages {
            assert!(!kp.is_empty());
        }
        // Each key package should be unique.
        for i in 0..packages.len() {
            for j in (i + 1)..packages.len() {
                assert_ne!(packages[i], packages[j], "key packages must be unique");
            }
        }
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
            DecryptedMessage::None | DecryptedMessage::Failed(_) => {}
            DecryptedMessage::Application(_) => {
                panic!("carol must NOT be able to decrypt after removal")
            }
            DecryptedMessage::Commit(_) => {
                panic!("carol must NOT process this as a commit")
            }
        }
    }

    // ── Multi-Epoch Decryption (RFC 9420 §14 epoch retention) ─────

    #[test]
    fn test_multi_epoch_decryption() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();
        assert_eq!(bob_group_id, group_id);

        let plaintext = b"message at epoch N";
        let ciphertext = alice.encrypt_message(&group_id, plaintext).unwrap();

        let (rotate_commit, _gi) = alice.rotate_keys(&group_id).unwrap();
        bob.decrypt_message(&bob_group_id, &rotate_commit).unwrap();

        let decrypted = bob.decrypt_message(&bob_group_id, &ciphertext).unwrap();
        match decrypted {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, plaintext.to_vec());
            }
            other => panic!("expected Application, got {other:?}"),
        }
    }

    #[test]
    fn test_multiple_key_rotations() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        for _ in 0..3 {
            let (rotate_commit, _gi) = alice.rotate_keys(&group_id).unwrap();
            bob.decrypt_message(&bob_group_id, &rotate_commit).unwrap();
        }

        let plaintext = b"after rotations";
        let ciphertext = alice.encrypt_message(&group_id, plaintext).unwrap();
        let decrypted = bob.decrypt_message(&bob_group_id, &ciphertext).unwrap();
        match decrypted {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, plaintext.to_vec());
            }
            other => panic!("expected Application, got {other:?}"),
        }
    }

    // ── Bidirectional Messaging ───────────────────────────────────

    #[test]
    fn test_bidirectional_messaging() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let ct1 = alice.encrypt_message(&group_id, b"hello bob").unwrap();
        let dec1 = bob.decrypt_message(&bob_group_id, &ct1).unwrap();
        match dec1 {
            DecryptedMessage::Application(data) => assert_eq!(data, b"hello bob"),
            other => panic!("expected Application, got {other:?}"),
        }

        let ct2 = bob.encrypt_message(&bob_group_id, b"hello alice").unwrap();
        let dec2 = alice.decrypt_message(&group_id, &ct2).unwrap();
        match dec2 {
            DecryptedMessage::Application(data) => assert_eq!(data, b"hello alice"),
            other => panic!("expected Application, got {other:?}"),
        }
    }

    #[test]
    fn test_sequential_messages_same_sender() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        for i in 0..5 {
            let msg = format!("message {i}");
            let ct = alice.encrypt_message(&group_id, msg.as_bytes()).unwrap();
            let dec = bob.decrypt_message(&bob_group_id, &ct).unwrap();
            match dec {
                DecryptedMessage::Application(data) => {
                    assert_eq!(data, msg.as_bytes().to_vec());
                }
                other => panic!("expected Application for message {i}, got {other:?}"),
            }
        }
    }

    // ── Message Content Tests ─────────────────────────────────────

    #[test]
    fn test_empty_message_roundtrip() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let ct = alice.encrypt_message(&group_id, b"").unwrap();
        let dec = bob.decrypt_message(&bob_group_id, &ct).unwrap();
        match dec {
            DecryptedMessage::Application(data) => assert!(data.is_empty()),
            other => panic!("expected empty Application, got {other:?}"),
        }
    }

    #[test]
    fn test_large_message_roundtrip() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let large_msg = vec![0x42u8; 64 * 1024];
        let ct = alice.encrypt_message(&group_id, &large_msg).unwrap();
        let dec = bob.decrypt_message(&bob_group_id, &ct).unwrap();
        match dec {
            DecryptedMessage::Application(data) => assert_eq!(data, large_msg),
            other => panic!("expected Application, got {other:?}"),
        }
    }

    #[test]
    fn test_unicode_content_roundtrip() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let unicode_msg = "Hello 🌍! Привет мир! こんにちは世界!";
        let ct = alice
            .encrypt_message(&group_id, unicode_msg.as_bytes())
            .unwrap();
        let dec = bob.decrypt_message(&bob_group_id, &ct).unwrap();
        match dec {
            DecryptedMessage::Application(data) => {
                assert_eq!(String::from_utf8(data).unwrap(), unicode_msg);
            }
            other => panic!("expected Application, got {other:?}"),
        }
    }

    // ── Forward Secrecy (RFC 9420 §16.1) ──────────────────────────

    #[test]
    fn test_forward_secrecy_removed_member_cannot_decrypt() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let bob_index = alice.find_member_index(&group_id, "bob").unwrap().unwrap();
        let (remove_commit, _gi) = alice.remove_member(&group_id, bob_index).unwrap();

        let result = bob.decrypt_message(&bob_group_id, &remove_commit).unwrap();
        match result {
            DecryptedMessage::Commit(info) => assert!(info.self_removed),
            other => panic!("expected Commit with self_removed, got {other:?}"),
        }

        let ct = alice
            .encrypt_message(&group_id, b"secret post-removal")
            .unwrap();

        let bob_result = bob.decrypt_message(&bob_group_id, &ct).unwrap();
        match bob_result {
            DecryptedMessage::Application(_) => {
                panic!("removed member must not decrypt post-removal messages")
            }
            DecryptedMessage::Failed(_) | DecryptedMessage::None => {}
            DecryptedMessage::Commit(_) => panic!("should not be a commit"),
        }
    }

    // ── Post-Compromise Security (RFC 9420 §16.2) ─────────────────

    #[test]
    fn test_post_compromise_security_via_rotation() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let (rotate_commit, _gi) = alice.rotate_keys(&group_id).unwrap();
        bob.decrypt_message(&bob_group_id, &rotate_commit).unwrap();

        let ct = alice
            .encrypt_message(&group_id, b"after key rotation")
            .unwrap();
        let dec = bob.decrypt_message(&bob_group_id, &ct).unwrap();
        match dec {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, b"after key rotation");
            }
            other => panic!("expected Application, got {other:?}"),
        }
    }

    // ── Three-Member Group ────────────────────────────────────────

    #[test]
    fn test_three_member_group_messaging() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");
        let (_dir_c, carol) = create_manager("carol");

        let bob_kp = bob.generate_key_package().unwrap();
        let carol_kp = carol.generate_key_package().unwrap();

        let mut members = HashMap::new();
        members.insert("bob".to_string(), bob_kp);
        members.insert("carol".to_string(), carol_kp);

        let (group_id, _commit, welcome_map, _gi) = alice.create_group(&members).unwrap();
        let bob_gid = bob.join_group(welcome_map.get("bob").unwrap()).unwrap();
        let carol_gid = carol.join_group(welcome_map.get("carol").unwrap()).unwrap();
        assert_eq!(bob_gid, group_id);
        assert_eq!(carol_gid, group_id);

        let ct = alice.encrypt_message(&group_id, b"hello all").unwrap();
        match bob.decrypt_message(&bob_gid, &ct).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, b"hello all"),
            other => panic!("bob: expected Application, got {other:?}"),
        }
        match carol.decrypt_message(&carol_gid, &ct).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, b"hello all"),
            other => panic!("carol: expected Application, got {other:?}"),
        }
    }

    // ── Sequential Member Addition ────────────────────────────────

    #[test]
    fn test_sequential_member_addition() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");
        let (_dir_c, carol) = create_manager("carol");

        let bob_kp = bob.generate_key_package().unwrap();
        let mut members = HashMap::new();
        members.insert("bob".to_string(), bob_kp);

        let (group_id, _commit, welcome_map, _gi) = alice.create_group(&members).unwrap();
        let bob_gid = bob.join_group(welcome_map.get("bob").unwrap()).unwrap();

        let carol_kp = carol.generate_key_package().unwrap();
        let mut carol_members = HashMap::new();
        carol_members.insert("carol".to_string(), carol_kp);

        let (invite_commit, carol_welcome_map, _gi) =
            alice.invite_to_group(&group_id, &carol_members).unwrap();

        bob.decrypt_message(&bob_gid, &invite_commit).unwrap();

        let carol_gid = carol
            .join_group(carol_welcome_map.get("carol").unwrap())
            .unwrap();
        assert_eq!(carol_gid, group_id);

        let ct = alice.encrypt_message(&group_id, b"trio message").unwrap();
        match bob.decrypt_message(&bob_gid, &ct).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, b"trio message"),
            other => panic!("bob: expected Application, got {other:?}"),
        }
        match carol.decrypt_message(&carol_gid, &ct).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, b"trio message"),
            other => panic!("carol: expected Application, got {other:?}"),
        }
    }

    // ── External Rejoin ───────────────────────────────────────────

    #[test]
    fn test_external_rejoin_without_removal() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, gi) = setup_alice_bob();

        // bob2 uses external commit to join (without removing old bob)
        let (_dir_b2, bob2) = create_manager("bob2");
        let (rejoin_gid, rejoin_commit) = bob2.external_rejoin_group(&gi, None).unwrap();
        assert_eq!(rejoin_gid, group_id);
        assert!(!rejoin_commit.is_empty());

        let result = alice.decrypt_message(&group_id, &rejoin_commit).unwrap();
        match result {
            DecryptedMessage::Commit(_) => {}
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    // ── Group Info Details ─────────────────────────────────────────

    #[test]
    fn test_group_info_after_member_add() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        bob.join_group(&welcome).unwrap();

        let details = alice.group_info_details(&group_id).unwrap();
        assert_eq!(details.member_count, 2);
        assert!(details.epoch > 0);

        let member_names: Vec<&str> = details.members.iter().map(|(_, n)| n.as_str()).collect();
        assert!(member_names.contains(&"alice"));
        assert!(member_names.contains(&"bob"));
    }

    // ── Key Package Properties ────────────────────────────────────

    #[test]
    fn test_key_package_uniqueness() {
        let (_dir, mgr) = create_manager("alice");
        let kp1 = mgr.generate_key_package().unwrap();
        let kp2 = mgr.generate_key_package().unwrap();
        assert_ne!(kp1, kp2, "each key package must be unique");
    }

    #[test]
    fn test_last_resort_differs_from_regular() {
        let (_dir, mgr) = create_manager("alice");
        let regular = mgr.generate_key_package().unwrap();
        let last_resort = mgr.generate_last_resort_key_package().unwrap();
        assert_ne!(
            regular, last_resort,
            "last-resort should differ from regular key package"
        );
    }

    // ── Cipher Suite Verification ─────────────────────────────────

    #[test]
    fn test_cipher_suite_matches_configured() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();
        let details = alice.group_info_details(&group_id).unwrap();
        // CIPHERSUITE constant resolves to CipherSuite(6), which is
        // the 256-bit security suite used by Conclave.
        let expected = format!("{:?}", CIPHERSUITE);
        assert_eq!(details.cipher_suite, expected);
    }

    // ── Self-Message Handling ─────────────────────────────────────

    #[test]
    fn test_decrypt_own_message_returns_failed_self() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        bob.join_group(&welcome).unwrap();

        let ct = alice.encrypt_message(&group_id, b"self-test").unwrap();
        let result = alice.decrypt_message(&group_id, &ct).unwrap();
        // mls-rs returns "message from self can't be processed" which our code
        // maps to either DecryptedMessage::None or DecryptedMessage::Failed
        match result {
            DecryptedMessage::None | DecryptedMessage::Failed(_) => {}
            other => panic!("decrypting own message should return None or Failed, got {other:?}"),
        }
    }

    // ── Data Directory ────────────────────────────────────────────

    #[test]
    fn test_manager_data_dir() {
        let (dir, mgr) = create_manager("alice");
        assert_eq!(mgr.data_dir(), dir.path());
    }

    // ── Invalid Welcome ───────────────────────────────────────────

    #[test]
    fn test_join_with_invalid_welcome_fails() {
        let (_dir, mgr) = create_manager("alice");
        let result = mgr.join_group(b"definitely not a valid welcome");
        assert!(result.is_err());
    }

    // ── Find Member Index ─────────────────────────────────────────

    #[test]
    fn test_find_member_index_for_self() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();
        let alice_index = alice.find_member_index(&group_id, "alice").unwrap();
        assert!(alice_index.is_some());
    }

    #[test]
    fn test_find_member_index_nonexistent_user() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();
        let result = alice.find_member_index(&group_id, "nonexistent").unwrap();
        assert!(result.is_none());
    }

    // ── Epoch Advances ────────────────────────────────────────────

    #[test]
    fn test_epoch_advances_on_add() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();
        let epoch_after_create = alice.group_info_details(&group_id).unwrap().epoch;

        let (_dir_c, carol) = create_manager("carol");
        let carol_kp = carol.generate_key_package().unwrap();
        let mut members = HashMap::new();
        members.insert("carol".to_string(), carol_kp);
        alice.invite_to_group(&group_id, &members).unwrap();

        let epoch_after_invite = alice.group_info_details(&group_id).unwrap().epoch;
        assert!(
            epoch_after_invite > epoch_after_create,
            "epoch must advance after adding a member"
        );
    }

    #[test]
    fn test_epoch_advances_on_removal() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        bob.join_group(&welcome).unwrap();

        let epoch_before = alice.group_info_details(&group_id).unwrap().epoch;
        let bob_index = alice.find_member_index(&group_id, "bob").unwrap().unwrap();
        alice.remove_member(&group_id, bob_index).unwrap();
        let epoch_after = alice.group_info_details(&group_id).unwrap().epoch;
        assert!(
            epoch_after > epoch_before,
            "epoch must advance after removing a member"
        );
    }

    #[test]
    fn test_epoch_advances_on_key_rotation() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();
        let epoch_before = alice.group_info_details(&group_id).unwrap().epoch;
        alice.rotate_keys(&group_id).unwrap();
        let epoch_after = alice.group_info_details(&group_id).unwrap().epoch;
        assert!(
            epoch_after > epoch_before,
            "epoch must advance after key rotation"
        );
    }

    // ── Delete / Wipe State ───────────────────────────────────────

    #[test]
    fn test_delete_group_then_load_fails() {
        let (_dir_a, alice, _dir_b, _bob, group_id, _commit, _welcome, _gi) = setup_alice_bob();
        alice.delete_group_state(&group_id).unwrap();
        let result = alice.group_info_details(&group_id);
        assert!(result.is_err(), "loading deleted group should fail");
    }

    #[test]
    fn test_wipe_then_recreate_identity() {
        let (dir, mgr) = create_manager("alice");
        mgr.wipe_local_state().unwrap();

        let mgr2 = MlsManager::new(dir.path(), "alice").unwrap();
        let kp = mgr2.generate_key_package().unwrap();
        assert!(!kp.is_empty());
    }

    // ── Message After Rotation Both Directions ────────────────────

    #[test]
    fn test_message_after_rotation_both_directions() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let (rotate_commit, _gi) = bob.rotate_keys(&bob_group_id).unwrap();
        alice.decrypt_message(&group_id, &rotate_commit).unwrap();

        let ct1 = alice.encrypt_message(&group_id, b"from alice").unwrap();
        match bob.decrypt_message(&bob_group_id, &ct1).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, b"from alice"),
            other => panic!("expected Application, got {other:?}"),
        }

        let ct2 = bob.encrypt_message(&bob_group_id, b"from bob").unwrap();
        match alice.decrypt_message(&group_id, &ct2).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, b"from bob"),
            other => panic!("expected Application, got {other:?}"),
        }
    }

    // ── Leave Group ───────────────────────────────────────────────

    #[test]
    fn test_leave_group_returns_commit() {
        let (_dir_a, _alice, _dir_b, bob, _group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let leave_result = bob.leave_group(&bob_group_id).unwrap();
        if let Some((commit_bytes, _gi_bytes)) = leave_result {
            assert!(!commit_bytes.is_empty());
        }
    }

    // ── Epoch Retention Boundary (RFC 9420 §14) ─────────────────

    #[test]
    fn test_epoch_retention_boundary() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        // Encrypt a message at the current epoch.
        let plaintext = b"old epoch message";
        let ciphertext = alice.encrypt_message(&group_id, plaintext).unwrap();

        // Advance the epoch more than EPOCH_RETENTION (16) times so the
        // old epoch is evicted from bob's state.
        for _ in 0..(EPOCH_RETENTION + 1) {
            let (rotate_commit, _gi) = alice.rotate_keys(&group_id).unwrap();
            bob.decrypt_message(&bob_group_id, &rotate_commit).unwrap();
        }

        // Bob should no longer be able to decrypt the message from the
        // evicted epoch.
        let result = bob.decrypt_message(&bob_group_id, &ciphertext).unwrap();
        match result {
            DecryptedMessage::Failed(_) => {}
            DecryptedMessage::Application(_) => {
                panic!("must not decrypt a message from an evicted epoch")
            }
            other => panic!(
                "expected Failed for evicted epoch, got {other:?}"
            ),
        }
    }

    // ── Five-Member Group ───────────────────────────────────────

    #[test]
    fn test_five_member_group() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");
        let (_dir_c, carol) = create_manager("carol");
        let (_dir_d, dave) = create_manager("dave");
        let (_dir_e, eve) = create_manager("eve");

        let bob_kp = bob.generate_key_package().unwrap();
        let carol_kp = carol.generate_key_package().unwrap();
        let dave_kp = dave.generate_key_package().unwrap();
        let eve_kp = eve.generate_key_package().unwrap();

        let mut members = HashMap::new();
        members.insert("bob".to_string(), bob_kp);
        members.insert("carol".to_string(), carol_kp);
        members.insert("dave".to_string(), dave_kp);
        members.insert("eve".to_string(), eve_kp);

        let (group_id, _commit, welcome_map, _gi) = alice.create_group(&members).unwrap();

        let bob_gid = bob.join_group(welcome_map.get("bob").unwrap()).unwrap();
        let carol_gid = carol.join_group(welcome_map.get("carol").unwrap()).unwrap();
        let dave_gid = dave.join_group(welcome_map.get("dave").unwrap()).unwrap();
        let eve_gid = eve.join_group(welcome_map.get("eve").unwrap()).unwrap();

        assert_eq!(bob_gid, group_id);
        assert_eq!(carol_gid, group_id);
        assert_eq!(dave_gid, group_id);
        assert_eq!(eve_gid, group_id);

        let details = alice.group_info_details(&group_id).unwrap();
        assert_eq!(
            details.member_count, 5,
            "group must have 5 members (alice + bob + carol + dave + eve)"
        );

        // Alice sends a message, all others can decrypt.
        let plaintext = b"hello five-member group";
        let ct = alice.encrypt_message(&group_id, plaintext).unwrap();

        for (name, mgr, gid) in [
            ("bob", &bob, &bob_gid),
            ("carol", &carol, &carol_gid),
            ("dave", &dave, &dave_gid),
            ("eve", &eve, &eve_gid),
        ] {
            match mgr.decrypt_message(gid, &ct).unwrap() {
                DecryptedMessage::Application(data) => {
                    assert_eq!(data, plaintext.to_vec(), "{name} must decrypt correctly");
                }
                other => panic!("{name}: expected Application, got {other:?}"),
            }
        }

        // Eve sends a message, alice and bob can decrypt.
        let eve_msg = b"message from eve";
        let eve_ct = eve.encrypt_message(&eve_gid, eve_msg).unwrap();

        match alice.decrypt_message(&group_id, &eve_ct).unwrap() {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, eve_msg.to_vec(), "alice must decrypt eve's message");
            }
            other => panic!("alice: expected Application from eve, got {other:?}"),
        }
        match bob.decrypt_message(&bob_gid, &eve_ct).unwrap() {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, eve_msg.to_vec(), "bob must decrypt eve's message");
            }
            other => panic!("bob: expected Application from eve, got {other:?}"),
        }
    }

    // ── Invite After Multiple Rotations ─────────────────────────

    #[test]
    fn test_invite_after_multiple_rotations() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        // Perform several key rotations before inviting a new member.
        for _ in 0..5 {
            let (rotate_commit, _gi) = alice.rotate_keys(&group_id).unwrap();
            bob.decrypt_message(&bob_group_id, &rotate_commit).unwrap();
        }

        let epoch_before_invite = alice.group_info_details(&group_id).unwrap().epoch;
        assert!(
            epoch_before_invite >= 6,
            "epoch must be at least 6 after 5 rotations and initial commit"
        );

        // Now invite carol.
        let (_dir_c, carol) = create_manager("carol");
        let carol_kp = carol.generate_key_package().unwrap();
        let mut carol_members = HashMap::new();
        carol_members.insert("carol".to_string(), carol_kp);

        let (invite_commit, carol_welcome_map, _gi) =
            alice.invite_to_group(&group_id, &carol_members).unwrap();

        bob.decrypt_message(&bob_group_id, &invite_commit).unwrap();

        let carol_gid = carol
            .join_group(carol_welcome_map.get("carol").unwrap())
            .unwrap();
        assert_eq!(carol_gid, group_id, "carol's group_id must match");

        // Verify all three can communicate.
        let plaintext = b"post-rotation invite test";
        let ct = alice.encrypt_message(&group_id, plaintext).unwrap();

        match bob.decrypt_message(&bob_group_id, &ct).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, plaintext.to_vec()),
            other => panic!("bob: expected Application, got {other:?}"),
        }
        match carol.decrypt_message(&carol_gid, &ct).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, plaintext.to_vec()),
            other => panic!("carol: expected Application, got {other:?}"),
        }
    }

    // ── Removed Member Cannot Rejoin via Welcome ────────────────

    #[test]
    fn test_removed_member_cannot_rejoin_via_welcome() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");
        let (_dir_c, carol) = create_manager("carol");

        let bob_kp = bob.generate_key_package().unwrap();
        let mut bob_members = HashMap::new();
        bob_members.insert("bob".to_string(), bob_kp);

        let (group_id, _commit, welcome_map_bob, _gi) = alice.create_group(&bob_members).unwrap();
        bob.join_group(welcome_map_bob.get("bob").unwrap()).unwrap();

        // Invite carol.
        let carol_kp = carol.generate_key_package().unwrap();
        let mut carol_members = HashMap::new();
        carol_members.insert("carol".to_string(), carol_kp);

        let (invite_commit, carol_welcome_map, _gi) =
            alice.invite_to_group(&group_id, &carol_members).unwrap();
        bob.decrypt_message(&group_id, &invite_commit).unwrap();
        carol
            .join_group(carol_welcome_map.get("carol").unwrap())
            .unwrap();

        // Remove carol.
        let carol_index = alice
            .find_member_index(&group_id, "carol")
            .unwrap()
            .expect("carol must be in the group");
        let (removal_commit, _gi) = alice.remove_member(&group_id, carol_index).unwrap();
        bob.decrypt_message(&group_id, &removal_commit).unwrap();

        // Carol tries to rejoin using her old welcome message, which should
        // fail because the key package was already consumed and state is stale.
        let old_welcome = carol_welcome_map.get("carol").unwrap();
        let rejoin_result = carol.join_group(old_welcome);
        assert!(
            rejoin_result.is_err(),
            "removed member must not be able to rejoin with an old welcome"
        );
    }

    // ── External Rejoin with Self-Removal ───────────────────────

    #[test]
    fn test_external_rejoin_with_self_removal() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, gi) = setup_alice_bob();
        bob.join_group(&welcome).unwrap();

        // Find bob's current leaf index.
        let bob_index = alice
            .find_member_index(&group_id, "bob")
            .unwrap()
            .expect("bob must be found in group");

        // Bob does an external rejoin, removing his old leaf.
        let (_dir_b2, bob_new) = create_manager("bob");
        let (rejoin_gid, rejoin_commit) = bob_new
            .external_rejoin_group(&gi, Some(bob_index))
            .unwrap();

        assert_eq!(
            rejoin_gid, group_id,
            "external rejoin must produce the same group_id"
        );
        assert!(
            !rejoin_commit.is_empty(),
            "external rejoin commit must not be empty"
        );

        // Alice processes the external commit.
        let result = alice.decrypt_message(&group_id, &rejoin_commit).unwrap();
        match result {
            DecryptedMessage::Commit(info) => {
                assert!(
                    !info.self_removed,
                    "alice must not be self_removed by bob's rejoin"
                );
            }
            other => panic!("expected Commit from external rejoin, got {other:?}"),
        }

        // Verify alice and the new bob can still communicate.
        let plaintext = b"after external rejoin";
        let ct = alice.encrypt_message(&group_id, plaintext).unwrap();
        match bob_new.decrypt_message(&rejoin_gid, &ct).unwrap() {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, plaintext.to_vec());
            }
            other => panic!("bob_new: expected Application, got {other:?}"),
        }
    }

    // ── Multiple Groups Isolation ───────────────────────────────

    #[test]
    fn test_multiple_groups_isolation() {
        let (_dir_a, alice) = create_manager("alice");
        let (_dir_b, bob) = create_manager("bob");

        // Create group 1 (alice + bob).
        let bob_kp1 = bob.generate_key_package().unwrap();
        let mut members1 = HashMap::new();
        members1.insert("bob".to_string(), bob_kp1);
        let (group1_id, _commit1, welcome_map1, _gi1) = alice.create_group(&members1).unwrap();
        let bob_gid1 = bob.join_group(welcome_map1.get("bob").unwrap()).unwrap();

        // Create group 2 (alice + bob).
        let bob_kp2 = bob.generate_key_package().unwrap();
        let mut members2 = HashMap::new();
        members2.insert("bob".to_string(), bob_kp2);
        let (group2_id, _commit2, welcome_map2, _gi2) = alice.create_group(&members2).unwrap();
        let bob_gid2 = bob.join_group(welcome_map2.get("bob").unwrap()).unwrap();

        assert_ne!(
            group1_id, group2_id,
            "two groups must have different IDs"
        );

        // Send a message in group 1.
        let msg1 = b"group 1 only";
        let ct1 = alice.encrypt_message(&group1_id, msg1).unwrap();

        // Bob can decrypt it in group 1.
        match bob.decrypt_message(&bob_gid1, &ct1).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, msg1.to_vec()),
            other => panic!("bob group1: expected Application, got {other:?}"),
        }

        // Bob cannot decrypt group 1's message in group 2.
        let cross_result = bob.decrypt_message(&bob_gid2, &ct1).unwrap();
        match cross_result {
            DecryptedMessage::Failed(_) => {}
            DecryptedMessage::Application(_) => {
                panic!("group 1 message must not be decryptable in group 2")
            }
            other => panic!("expected Failed for cross-group decrypt, got {other:?}"),
        }

        // Send a message in group 2 and verify it works there.
        let msg2 = b"group 2 only";
        let ct2 = alice.encrypt_message(&group2_id, msg2).unwrap();
        match bob.decrypt_message(&bob_gid2, &ct2).unwrap() {
            DecryptedMessage::Application(data) => assert_eq!(data, msg2.to_vec()),
            other => panic!("bob group2: expected Application, got {other:?}"),
        }
    }

    // ── Rapid Sequential Messages ───────────────────────────────

    #[test]
    fn test_rapid_sequential_messages() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        let message_count = 50;
        let mut ciphertexts = Vec::with_capacity(message_count);

        // Encrypt many messages in quick succession.
        for i in 0..message_count {
            let msg = format!("rapid message {i}");
            let ct = alice.encrypt_message(&group_id, msg.as_bytes()).unwrap();
            ciphertexts.push((msg, ct));
        }

        // Decrypt all of them in order.
        for (expected_msg, ct) in &ciphertexts {
            match bob.decrypt_message(&bob_group_id, ct).unwrap() {
                DecryptedMessage::Application(data) => {
                    assert_eq!(
                        data,
                        expected_msg.as_bytes().to_vec(),
                        "mismatch for '{expected_msg}'"
                    );
                }
                other => panic!("expected Application for '{expected_msg}', got {other:?}"),
            }
        }
    }

    // ── Binary Payload Roundtrip ────────────────────────────────

    #[test]
    fn test_binary_payload_roundtrip() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        // Construct non-UTF8 binary data (all 256 byte values).
        let binary_payload: Vec<u8> = (0..=255).collect();
        assert!(
            std::str::from_utf8(&binary_payload).is_err(),
            "test data must not be valid UTF-8"
        );

        let ct = alice.encrypt_message(&group_id, &binary_payload).unwrap();
        match bob.decrypt_message(&bob_group_id, &ct).unwrap() {
            DecryptedMessage::Application(data) => {
                assert_eq!(
                    data, binary_payload,
                    "binary payload must survive encrypt/decrypt roundtrip"
                );
            }
            other => panic!("expected Application for binary payload, got {other:?}"),
        }
    }

    // ── Leave Group Self-Removal Detection ──────────────────────

    #[test]
    fn test_leave_group_self_removal_detection() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        // Alice removes bob. When bob processes the removal commit, the
        // CommitEffect should be `Removed` which sets self_removed = true.
        let bob_index = alice
            .find_member_index(&group_id, "bob")
            .unwrap()
            .expect("bob must be found in group");
        let (removal_commit, _gi) = alice.remove_member(&group_id, bob_index).unwrap();

        let result = bob.decrypt_message(&bob_group_id, &removal_commit).unwrap();
        match result {
            DecryptedMessage::Commit(info) => {
                assert!(
                    info.self_removed,
                    "bob must see self_removed = true when he is removed"
                );
            }
            other => panic!("expected Commit with self_removed for bob, got {other:?}"),
        }

        // Also test the leave_group path: bob tries to leave on his own.
        // mls-rs may or may not support self-removal commits; if it does,
        // alice should see it as a removal commit.
        let (_dir_a2, alice2) = create_manager("alice");
        let (_dir_b2, bob2) = create_manager("bob");

        let bob2_kp = bob2.generate_key_package().unwrap();
        let mut members = HashMap::new();
        members.insert("bob".to_string(), bob2_kp);
        let (group2_id, _commit2, welcome_map2, _gi2) = alice2.create_group(&members).unwrap();
        let bob2_gid = bob2.join_group(welcome_map2.get("bob").unwrap()).unwrap();

        let leave_result = bob2.leave_group(&bob2_gid).unwrap();
        if let Some((leave_commit, _gi)) = leave_result {
            assert!(!leave_commit.is_empty(), "leave commit must not be empty");

            let alice_result = alice2.decrypt_message(&group2_id, &leave_commit).unwrap();
            match alice_result {
                DecryptedMessage::Commit(info) => {
                    assert!(
                        !info.self_removed,
                        "alice must not be self_removed when bob leaves"
                    );
                    assert!(
                        !info.members_removed.is_empty(),
                        "the commit must report a member removal"
                    );
                }
                other => panic!("expected Commit for bob's leave, got {other:?}"),
            }
        }
    }

    // ── Group Info Epoch Matches ────────────────────────────────

    #[test]
    fn test_group_info_epoch_matches() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        // Track epoch through several operations.
        let epoch_after_create = alice.group_info_details(&group_id).unwrap().epoch;

        // Key rotation.
        let (rotate_commit, _gi) = alice.rotate_keys(&group_id).unwrap();
        bob.decrypt_message(&bob_group_id, &rotate_commit).unwrap();
        let epoch_after_rotation = alice.group_info_details(&group_id).unwrap().epoch;
        assert_eq!(
            epoch_after_rotation,
            epoch_after_create + 1,
            "epoch must increment by 1 after rotation"
        );

        // Invite carol.
        let (_dir_c, carol) = create_manager("carol");
        let carol_kp = carol.generate_key_package().unwrap();
        let mut carol_members = HashMap::new();
        carol_members.insert("carol".to_string(), carol_kp);
        let (invite_commit, _carol_welcome, _gi) =
            alice.invite_to_group(&group_id, &carol_members).unwrap();
        bob.decrypt_message(&bob_group_id, &invite_commit).unwrap();
        let epoch_after_invite = alice.group_info_details(&group_id).unwrap().epoch;
        assert_eq!(
            epoch_after_invite,
            epoch_after_rotation + 1,
            "epoch must increment by 1 after invite"
        );

        // Remove carol.
        let carol_index = alice
            .find_member_index(&group_id, "carol")
            .unwrap()
            .expect("carol must be in the group");
        let (removal_commit, _gi) = alice.remove_member(&group_id, carol_index).unwrap();
        bob.decrypt_message(&bob_group_id, &removal_commit).unwrap();
        let epoch_after_removal = alice.group_info_details(&group_id).unwrap().epoch;
        assert_eq!(
            epoch_after_removal,
            epoch_after_invite + 1,
            "epoch must increment by 1 after removal"
        );

        // Verify bob's view of the epoch matches alice's.
        let bob_epoch = bob.group_info_details(&bob_group_id).unwrap().epoch;
        assert_eq!(
            bob_epoch, epoch_after_removal,
            "bob's epoch must match alice's epoch"
        );
    }

    // ── Concurrent Key Rotations from Different Members ─────────

    #[test]
    fn test_concurrent_key_rotations_from_different_members() {
        let (_dir_a, alice, _dir_b, bob, group_id, _commit, welcome, _gi) = setup_alice_bob();
        let bob_group_id = bob.join_group(&welcome).unwrap();

        // Alice rotates keys; bob processes the commit.
        let (alice_rotate_commit, _gi) = alice.rotate_keys(&group_id).unwrap();
        bob.decrypt_message(&bob_group_id, &alice_rotate_commit)
            .unwrap();

        // Bob rotates keys; alice processes the commit.
        let (bob_rotate_commit, _gi) = bob.rotate_keys(&bob_group_id).unwrap();
        alice
            .decrypt_message(&group_id, &bob_rotate_commit)
            .unwrap();

        // Both should be at the same epoch.
        let alice_epoch = alice.group_info_details(&group_id).unwrap().epoch;
        let bob_epoch = bob.group_info_details(&bob_group_id).unwrap().epoch;
        assert_eq!(
            alice_epoch, bob_epoch,
            "alice and bob must agree on epoch after mutual rotations"
        );

        // Verify bidirectional messaging still works.
        let ct1 = alice
            .encrypt_message(&group_id, b"alice after rotations")
            .unwrap();
        match bob.decrypt_message(&bob_group_id, &ct1).unwrap() {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, b"alice after rotations");
            }
            other => panic!("bob: expected Application, got {other:?}"),
        }

        let ct2 = bob
            .encrypt_message(&bob_group_id, b"bob after rotations")
            .unwrap();
        match alice.decrypt_message(&group_id, &ct2).unwrap() {
            DecryptedMessage::Application(data) => {
                assert_eq!(data, b"bob after rotations");
            }
            other => panic!("alice: expected Application, got {other:?}"),
        }
    }
}
