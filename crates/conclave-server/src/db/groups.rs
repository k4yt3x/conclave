use rusqlite::{params, OptionalExtension};

use crate::db::UserGroupRow;
use crate::error::{Error, Result};
use crate::validation::{validate_alias, validate_group_name};

use super::Database;

impl Database {
    pub fn group_exists(&self, group_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT 1 FROM groups WHERE id = ?1")?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id], |row| row.get(0))
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn create_group(
        &self,
        group_name: &str,
        alias: Option<&str>,
        creator_id: i64,
    ) -> Result<i64> {
        validate_group_name(group_name)?;
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO groups (group_name, alias) VALUES (?1, ?2)",
            params![group_name, alias],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(format!("group_name '{group_name}' already exists"))
            }
            other => Error::Database(other),
        })?;
        let group_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO group_members (group_id, user_id, role) VALUES (?1, ?2, 'admin')",
            params![group_id, creator_id],
        )?;
        Ok(group_id)
    }

    pub fn add_group_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO group_members (group_id, user_id) VALUES (?1, ?2)",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    pub fn is_group_member(&self, group_id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt =
            conn.prepare("SELECT 1 FROM group_members WHERE group_id = ?1 AND user_id = ?2")?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id, user_id], |row| row.get(0))
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn list_user_groups(&self, user_id: i64) -> Result<Vec<UserGroupRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT g.id, g.group_name, g.alias, g.created_at, g.mls_group_id,
                    g.message_expiry_seconds
             FROM groups g
             JOIN group_members gm ON g.id = gm.group_id
             WHERE gm.user_id = ?1
             ORDER BY g.created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![user_id], |row| {
                Ok(UserGroupRow {
                    group_id: row.get(0)?,
                    group_name: row.get(1)?,
                    alias: row.get(2)?,
                    created_at: row.get(3)?,
                    mls_group_id: row.get(4)?,
                    message_expiry_seconds: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn set_mls_group_id(&self, group_id: i64, mls_group_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE groups SET mls_group_id = ?2 WHERE id = ?1 AND mls_group_id IS NULL",
            params![group_id, mls_group_id],
        )?;
        Ok(())
    }

    pub fn remove_group_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    pub fn get_group_members(&self, group_id: i64) -> Result<Vec<crate::db::GroupMemberRow>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.alias, gm.role
             FROM users u
             JOIN group_members gm ON u.id = gm.user_id
             WHERE gm.group_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![group_id], |row| {
                Ok(crate::db::GroupMemberRow {
                    user_id: row.get(0)?,
                    username: row.get(1)?,
                    alias: row.get(2)?,
                    role: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Get the name for a group.
    pub fn get_group_name(&self, group_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let result: Option<String> = conn
            .prepare("SELECT group_name FROM groups WHERE id = ?1")?
            .query_row(params![group_id], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    pub fn get_group_alias(&self, group_id: i64) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare("SELECT alias FROM groups WHERE id = ?1")?;
        let result: Option<Option<String>> = stmt
            .query_row(params![group_id], |row| row.get(0))
            .optional()?;
        Ok(result.flatten())
    }

    pub fn update_group_alias(&self, group_id: i64, alias: Option<&str>) -> Result<()> {
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE groups SET alias = ?1 WHERE id = ?2",
            params![alias, group_id],
        )?;
        Ok(())
    }

    pub fn update_group_name(&self, group_id: i64, group_name: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE groups SET group_name = ?1 WHERE id = ?2",
            params![group_name, group_id],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(format!(
                    "group_name '{}' already exists",
                    group_name.unwrap_or_default()
                ))
            }
            other => Error::Database(other),
        })?;
        Ok(())
    }

    /// Get the message expiry setting for a group.
    pub fn get_group_expiry(&self, group_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let expiry: i64 = conn
            .prepare("SELECT message_expiry_seconds FROM groups WHERE id = ?1")?
            .query_row(params![group_id], |row| row.get(0))
            .optional()?
            .unwrap_or(-1);
        Ok(expiry)
    }

    /// Set the message expiry setting for a group.
    pub fn set_group_expiry(&self, group_id: i64, seconds: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE groups SET message_expiry_seconds = ?2 WHERE id = ?1",
            params![group_id, seconds],
        )?;
        Ok(())
    }

    /// Check whether a user is an admin of a group.
    pub fn is_group_admin(&self, group_id: i64, user_id: i64) -> Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT 1 FROM group_members WHERE group_id = ?1 AND user_id = ?2 AND role = 'admin'",
        )?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id, user_id], |row| row.get(0))
            .optional()?;
        Ok(exists.is_some())
    }

    /// Promote a member to admin.
    pub fn promote_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE group_members SET role = 'admin' WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    /// Demote an admin to regular member.
    pub fn demote_member(&self, group_id: i64, user_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE group_members SET role = 'member' WHERE group_id = ?1 AND user_id = ?2",
            params![group_id, user_id],
        )?;
        Ok(())
    }

    /// Count the number of admins in a group.
    pub fn count_group_admins(&self, group_id: i64) -> Result<i64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM group_members WHERE group_id = ?1 AND role = 'admin'",
            params![group_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// List admin members of a group: (user_id, username, alias).
    pub fn get_group_admins(&self, group_id: i64) -> Result<Vec<(i64, String, Option<String>)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.alias
             FROM users u
             JOIN group_members gm ON u.id = gm.user_id
             WHERE gm.group_id = ?1 AND gm.role = 'admin'",
        )?;
        let rows = stmt
            .query_map(params![group_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
