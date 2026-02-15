use std::collections::HashMap;
use std::path::Path;

use mls_rs::client_builder::MlsConfig;
use mls_rs::group::ReceivedMessage;
use mls_rs::identity::SigningIdentity;
use mls_rs::identity::basic::{BasicCredential, BasicIdentityProvider};
use mls_rs::{CipherSuite, CipherSuiteProvider, Client, CryptoProvider, ExtensionList, MlsMessage};
use mls_rs_crypto_openssl::OpensslCryptoProvider;
use mls_rs_provider_sqlite::SqLiteDataStorageEngine;
use mls_rs_provider_sqlite::connection_strategy::FileConnectionStrategy;

use crate::error::{Error, Result};

const CIPHERSUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

/// Persistent MLS state manager for the client.
pub struct MlsManager {
    identity_bytes: Vec<u8>,
    signing_key_bytes: Vec<u8>,
    data_dir: std::path::PathBuf,
}

impl MlsManager {
    /// Load or create MLS identity for the given username.
    ///
    /// Each user gets their own subdirectory under `data_dir` so that
    /// multiple users sharing the same client data directory do not
    /// collide on MLS identity files or SQLite state.
    pub fn new(data_dir: &Path, username: &str) -> Result<Self> {
        let user_dir = data_dir.join("users").join(username);
        std::fs::create_dir_all(&user_dir)?;

        let identity_path = user_dir.join("mls_identity.bin");
        let signing_key_path = user_dir.join("mls_signing_key.bin");

        if identity_path.exists() && signing_key_path.exists() {
            let identity_bytes = std::fs::read(&identity_path)?;
            let signing_key_bytes = std::fs::read(&signing_key_path)?;
            Ok(Self {
                identity_bytes,
                signing_key_bytes,
                data_dir: user_dir,
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

            Ok(Self {
                identity_bytes,
                signing_key_bytes,
                data_dir: user_dir,
            })
        }
    }

    /// Returns the per-user data directory (for storing group mappings, etc.).
    pub fn user_data_dir(&self) -> &Path {
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
            .group_info_message(true)
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
            .group_info_message(true)
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
    /// Returns Some(plaintext) for application messages, None for other message types (commits, etc.).
    pub fn decrypt_message(
        &self,
        mls_group_id: &str,
        mls_message_bytes: &[u8],
    ) -> Result<Option<Vec<u8>>> {
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
                return Ok(None);
            }
        };

        group
            .write_to_storage()
            .map_err(|e| Error::Mls(format!("write group state failed: {e}")))?;

        match received {
            ReceivedMessage::ApplicationMessage(app_msg) => Ok(Some(app_msg.data().to_vec())),
            ReceivedMessage::Commit(_)
            | ReceivedMessage::Proposal(_)
            | ReceivedMessage::GroupInfo(_)
            | ReceivedMessage::Welcome
            | ReceivedMessage::KeyPackage(_) => Ok(None),
        }
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
