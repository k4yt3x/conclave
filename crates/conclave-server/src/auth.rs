use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use std::sync::LazyLock;

use uuid::Uuid;

use crate::error::{Error, Result};
use crate::state::AppState;

/// Precomputed Argon2id hash used for timing equalization when a login
/// attempt targets a non-existent user. This ensures both the valid-user
/// and invalid-user code paths execute `verify_password` with identical
/// computational cost, preventing username enumeration via timing.
static DUMMY_HASH: LazyLock<String> =
    LazyLock::new(|| hash_password("timing_equalization_dummy").expect("dummy hash generation"));

/// Hash a password with Argon2id.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| Error::Internal(format!("password hashing failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify a password against an Argon2id hash.
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|e| Error::Internal(format!("invalid password hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

/// Return a reference to the precomputed dummy hash for timing equalization.
pub fn dummy_hash() -> &'static str {
    &DUMMY_HASH
}

/// Generate a cryptographically secure opaque token (256-bit, hex-encoded).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Axum extractor that validates the Bearer token and provides the authenticated user ID.
pub struct AuthUser {
    pub user_id: Uuid,
    pub token: String,
}

impl FromRequestParts<Arc<AppState>> for AuthUser {
    type Rejection = Error;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> std::result::Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Error::Unauthorized("missing Authorization header".into()))?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| Error::Unauthorized("invalid Authorization header format".into()))?;

        let user_id = state
            .db
            .validate_session(token)?
            .ok_or_else(|| Error::Unauthorized("invalid or expired token".into()))?;

        let new_expires_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            + state.config.token_ttl_seconds;
        state.db.extend_session(token, new_expires_at)?;

        Ok(AuthUser {
            user_id,
            token: token.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify_password() {
        let hash = hash_password("correcthorse").expect("hashing should succeed");
        let result = verify_password("correcthorse", &hash).expect("verification should succeed");
        assert!(result, "correct password should verify as true");
    }

    #[test]
    fn test_verify_wrong_password() {
        let hash = hash_password("correcthorse").expect("hashing should succeed");
        let result = verify_password("wrongpassword", &hash).expect("verification should succeed");
        assert!(!result, "wrong password should verify as false");
    }

    #[test]
    fn test_hash_produces_different_outputs() {
        let hash1 = hash_password("samepassword").expect("hashing should succeed");
        let hash2 = hash_password("samepassword").expect("hashing should succeed");
        assert_ne!(
            hash1, hash2,
            "two hashes of the same password should differ due to random salt"
        );
    }

    #[test]
    fn test_generate_token_uniqueness() {
        let token1 = generate_token();
        let token2 = generate_token();
        assert_ne!(token1, token2, "two generated tokens should differ");
    }

    #[test]
    fn test_generate_token_length() {
        let token = generate_token();
        assert_eq!(
            token.len(),
            64,
            "token should be 64 chars long (32 bytes hex-encoded)"
        );
    }

    #[test]
    fn test_verify_invalid_hash_format() {
        let result = verify_password("pass", "not_a_valid_hash");
        assert!(
            result.is_err(),
            "verifying against an invalid hash format should return Err"
        );
    }

    #[test]
    fn test_dummy_hash_is_valid() {
        let hash = dummy_hash();
        assert!(
            hash.starts_with("$argon2id$"),
            "dummy hash should be a valid Argon2id hash, got: {hash}"
        );
    }

    #[test]
    fn test_generate_token_is_hex() {
        let token = generate_token();
        assert!(
            hex::decode(&token).is_ok(),
            "token should be valid hexadecimal, got: {token}"
        );
    }

    #[test]
    fn test_hash_password_empty_input() {
        let result = hash_password("");
        assert!(
            result.is_ok(),
            "hashing an empty string should not panic or error"
        );
    }

    #[test]
    fn test_verify_password_empty_password() {
        let hash = hash_password("real_password").expect("hashing should succeed");
        let result = verify_password("", &hash).expect("verification should succeed");
        assert!(
            !result,
            "empty password should not verify against a real hash"
        );
    }
}
