use crate::error::{Error, Result};

const MAX_ALIAS_LENGTH: usize = 64;
const MAX_USERNAME_LENGTH: usize = 64;
const MAX_GROUP_NAME_LENGTH: usize = 64;

/// Check whether a username is valid.
pub fn validate_username(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Validation("username is required".to_string()));
    }
    if name.len() > MAX_USERNAME_LENGTH {
        return Err(Error::Validation(format!(
            "username exceeds maximum length of {MAX_USERNAME_LENGTH} characters"
        )));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !name.starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        return Err(Error::Validation(
            "username must start with a letter or digit and contain only ASCII letters, digits, and underscores".to_string(),
        ));
    }
    Ok(())
}

/// Check whether a group name is valid (same rules as username).
pub fn validate_group_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::Validation("group name is required".to_string()));
    }
    if name.len() > MAX_GROUP_NAME_LENGTH {
        return Err(Error::Validation(format!(
            "group name exceeds maximum length of {MAX_GROUP_NAME_LENGTH} characters"
        )));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || !name.starts_with(|c: char| c.is_ascii_alphanumeric())
    {
        return Err(Error::Validation(
            "group name must start with a letter or digit and contain only ASCII letters, digits, and underscores".to_string(),
        ));
    }
    Ok(())
}

const MIN_PASSWORD_LENGTH: usize = 8;

/// Check whether a password is valid (at least 8 characters).
pub fn validate_password(password: &str) -> Result<()> {
    if password.is_empty() {
        return Err(Error::Validation("password is required".to_string()));
    }
    if password.len() < MIN_PASSWORD_LENGTH {
        return Err(Error::Validation(format!(
            "password must be at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }
    Ok(())
}

/// Check whether an alias string is valid (no ASCII control characters, max 64 chars).
pub fn validate_alias(alias: &str) -> Result<()> {
    if alias.len() > MAX_ALIAS_LENGTH {
        return Err(Error::Validation(format!(
            "alias exceeds maximum length of {MAX_ALIAS_LENGTH} characters"
        )));
    }
    if alias.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(Error::Validation(
            "alias must not contain ASCII control characters".to_string(),
        ));
    }
    Ok(())
}
