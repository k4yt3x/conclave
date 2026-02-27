use rusqlite::{params, OptionalExtension};

use crate::db::UserRow;
use crate::error::{Error, Result};
use crate::validation::validate_alias;

use super::Database;

impl Database {
    pub fn create_user(&self, username: &str, password_hash: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO users (username, password_hash) VALUES (?1, ?2)",
            params![username, password_hash],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(format!("username '{username}' already exists"))
            }
            other => Error::Database(other),
        })?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<Option<UserRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare("SELECT id, username, password_hash, alias FROM users WHERE username = ?1")?;
        let result = stmt
            .query_row(params![username], |row| {
                Ok(UserRow {
                    user_id: row.get(0)?,
                    username: row.get(1)?,
                    password_hash: row.get(2)?,
                    alias: row.get(3)?,
                })
            })
            .optional()?;
        Ok(result)
    }

    pub fn get_user_by_id(&self, user_id: i64) -> Result<Option<(i64, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT id, username, alias FROM users WHERE id = ?1")?;
        let result = stmt
            .query_row(params![user_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?;
        Ok(result)
    }

    pub fn get_password_hash(&self, user_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT password_hash FROM users WHERE id = ?1")?;
        let result = stmt
            .query_row(params![user_id], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    pub fn get_user_id_by_username(&self, username: &str) -> Result<Option<i64>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT id FROM users WHERE username = ?1")?;
        let result = stmt
            .query_row(params![username], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    pub fn update_user_password(&self, user_id: i64, password_hash: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE users SET password_hash = ?1 WHERE id = ?2",
            params![password_hash, user_id],
        )?;
        Ok(())
    }

    pub fn update_user_alias(&self, user_id: i64, alias: Option<&str>) -> Result<()> {
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE users SET alias = ?1 WHERE id = ?2",
            params![alias, user_id],
        )?;
        Ok(())
    }
}
