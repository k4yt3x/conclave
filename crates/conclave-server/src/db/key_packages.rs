use rusqlite::{params, OptionalExtension};

use crate::error::Result;

use super::Database;

impl Database {
    /// Maximum number of regular (non-last-resort) key packages per user.
    pub(crate) const MAX_KEY_PACKAGES_PER_USER: i64 = 10;

    /// Store a key package for a user.
    ///
    /// If `is_last_resort` is true, any existing last-resort package for this
    /// user is replaced (at most one last-resort package per user).  Regular
    /// packages accumulate up to [`MAX_KEY_PACKAGES_PER_USER`](Self::MAX_KEY_PACKAGES_PER_USER).
    pub fn store_key_package(&self, user_id: i64, data: &[u8], is_last_resort: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        if is_last_resort {
            conn.execute(
                "DELETE FROM key_packages WHERE user_id = ?1 AND is_last_resort = 1",
                params![user_id],
            )?;
        } else {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM key_packages WHERE user_id = ?1 AND is_last_resort = 0",
                params![user_id],
                |row| row.get(0),
            )?;
            if count >= Self::MAX_KEY_PACKAGES_PER_USER {
                return Ok(());
            }
        }

        conn.execute(
            "INSERT INTO key_packages (user_id, key_package_data, is_last_resort) \
             VALUES (?1, ?2, ?3)",
            params![user_id, data, is_last_resort as i32],
        )?;
        Ok(())
    }

    /// Consume a key package for the given user.
    ///
    /// Prefers regular (non-last-resort) packages and deletes them on consumption.
    /// Falls back to the last-resort package if no regular ones remain — the
    /// last-resort package is returned but **not** deleted.
    pub fn consume_key_package(&self, user_id: i64) -> Result<Option<Vec<u8>>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        let mut stmt = conn.prepare(
            "SELECT id, key_package_data FROM key_packages \
             WHERE user_id = ?1 AND is_last_resort = 0 \
             ORDER BY created_at ASC LIMIT 1",
        )?;
        let regular: Option<(i64, Vec<u8>)> = stmt
            .query_row(params![user_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()?;

        if let Some((id, data)) = regular {
            conn.execute("DELETE FROM key_packages WHERE id = ?1", params![id])?;
            return Ok(Some(data));
        }

        let mut stmt = conn.prepare(
            "SELECT key_package_data FROM key_packages \
             WHERE user_id = ?1 AND is_last_resort = 1 LIMIT 1",
        )?;
        let last_resort: Option<Vec<u8>> = stmt
            .query_row(params![user_id], |row| row.get(0))
            .optional()?;

        Ok(last_resort)
    }

    /// Count key packages for a user: (regular, last_resort).
    pub fn count_key_packages(&self, user_id: i64) -> Result<(i64, i64)> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let regular: i64 = conn.query_row(
            "SELECT COUNT(*) FROM key_packages WHERE user_id = ?1 AND is_last_resort = 0",
            params![user_id],
            |row| row.get(0),
        )?;
        let last_resort: i64 = conn.query_row(
            "SELECT COUNT(*) FROM key_packages WHERE user_id = ?1 AND is_last_resort = 1",
            params![user_id],
            |row| row.get(0),
        )?;
        Ok((regular, last_resort))
    }

    pub fn delete_key_packages(&self, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM key_packages WHERE user_id = ?1",
            params![user_id],
        )?;
        Ok(())
    }
}
