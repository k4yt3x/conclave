use rusqlite::{OptionalExtension, params};
use uuid::Uuid;

use crate::db::{UserGroupRow, UserInfo};
use crate::error::{Error, Result};
use crate::validation::{validate_alias, validate_group_name};

use super::Database;

impl Database {
    pub fn group_exists(&self, group_id: Uuid) -> Result<bool> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare("SELECT 1 FROM groups WHERE id = ?1")?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id.to_string()], |row| row.get(0))
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn create_group(
        &self,
        group_name: &str,
        alias: Option<&str>,
        creator_id: Uuid,
    ) -> Result<Uuid> {
        validate_group_name(group_name)?;
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let group_id = Uuid::new_v4();
        let conn = self.lock_conn();
        conn.execute(
            "INSERT INTO groups (id, group_name, alias) VALUES (?1, ?2, ?3)",
            params![group_id.to_string(), group_name, alias],
        )
        .map_err(|e| match e {
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Error::Conflict(format!("group_name '{group_name}' already exists"))
            }
            other => Error::Database(other),
        })?;
        conn.execute(
            "INSERT INTO group_members (group_id, user_id, role) VALUES (?1, ?2, 'admin')",
            params![group_id.to_string(), creator_id.to_string()],
        )?;
        Ok(group_id)
    }

    pub fn add_group_member(&self, group_id: Uuid, user_id: Uuid) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "INSERT OR IGNORE INTO group_members (group_id, user_id) VALUES (?1, ?2)",
            params![group_id.to_string(), user_id.to_string()],
        )?;
        Ok(())
    }

    pub fn is_group_member(&self, group_id: Uuid, user_id: Uuid) -> Result<bool> {
        let conn = self.lock_conn();
        let mut stmt =
            conn.prepare("SELECT 1 FROM group_members WHERE group_id = ?1 AND user_id = ?2")?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id.to_string(), user_id.to_string()], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(exists.is_some())
    }

    pub fn list_user_groups(&self, user_id: Uuid) -> Result<Vec<UserGroupRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT g.id, g.group_name, g.alias, g.mls_group_id,
                    g.message_expiry_seconds, g.visibility
             FROM groups g
             JOIN group_members gm ON g.id = gm.group_id
             WHERE gm.user_id = ?1
             ORDER BY g.group_name ASC",
        )?;
        let rows = stmt
            .query_map(params![user_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                Ok((
                    id_str,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })?
            .collect::<std::result::Result<
                Vec<(String, String, Option<String>, Option<String>, i64, i32)>,
                _,
            >>()?;
        let mut result = Vec::with_capacity(rows.len());
        for (id_str, group_name, alias, mls_group_id, message_expiry_seconds, visibility) in rows {
            let group_id = Uuid::parse_str(&id_str)
                .map_err(|e| Error::Internal(format!("invalid group UUID: {e}")))?;
            result.push(UserGroupRow {
                group_id,
                group_name,
                alias,
                mls_group_id,
                message_expiry_seconds,
                visibility,
            });
        }
        Ok(result)
    }

    pub fn set_mls_group_id(&self, group_id: Uuid, mls_group_id: &str) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE groups SET mls_group_id = ?2 WHERE id = ?1 AND mls_group_id IS NULL",
            params![group_id.to_string(), mls_group_id],
        )?;
        Ok(())
    }

    pub fn remove_group_member(&self, group_id: Uuid, user_id: Uuid) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "DELETE FROM group_members WHERE group_id = ?1 AND user_id = ?2",
            params![group_id.to_string(), user_id.to_string()],
        )?;
        Ok(())
    }

    pub fn get_group_members(&self, group_id: Uuid) -> Result<Vec<crate::db::GroupMemberRow>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.alias, gm.role, u.signing_key_fingerprint
             FROM users u
             JOIN group_members gm ON u.id = gm.user_id
             WHERE gm.group_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![group_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                Ok((id_str, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?
            .collect::<std::result::Result<Vec<(String, String, Option<String>, String, Option<String>)>, _>>()?;
        let mut result = Vec::with_capacity(rows.len());
        for (id_str, username, alias, role, signing_key_fingerprint) in rows {
            let user_id = Uuid::parse_str(&id_str)
                .map_err(|e| Error::Internal(format!("invalid user UUID: {e}")))?;
            result.push(crate::db::GroupMemberRow {
                user_id,
                username,
                alias,
                role,
                signing_key_fingerprint,
            });
        }
        Ok(result)
    }

    /// Get the name for a group.
    pub fn get_group_name(&self, group_id: Uuid) -> Result<Option<String>> {
        let conn = self.lock_conn();
        let result: Option<String> = conn
            .prepare("SELECT group_name FROM groups WHERE id = ?1")?
            .query_row(params![group_id.to_string()], |row| row.get(0))
            .optional()?;
        Ok(result)
    }

    pub fn get_group_alias(&self, group_id: Uuid) -> Result<Option<String>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare("SELECT alias FROM groups WHERE id = ?1")?;
        let result: Option<Option<String>> = stmt
            .query_row(params![group_id.to_string()], |row| row.get(0))
            .optional()?;
        Ok(result.flatten())
    }

    pub fn update_group_alias(&self, group_id: Uuid, alias: Option<&str>) -> Result<()> {
        if let Some(alias) = alias {
            validate_alias(alias)?;
        }
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE groups SET alias = ?1 WHERE id = ?2",
            params![alias, group_id.to_string()],
        )?;
        Ok(())
    }

    pub fn update_group_name(&self, group_id: Uuid, group_name: Option<&str>) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE groups SET group_name = ?1 WHERE id = ?2",
            params![group_name, group_id.to_string()],
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
    pub fn get_group_expiry(&self, group_id: Uuid) -> Result<i64> {
        let conn = self.lock_conn();
        let expiry: i64 = conn
            .prepare("SELECT message_expiry_seconds FROM groups WHERE id = ?1")?
            .query_row(params![group_id.to_string()], |row| row.get(0))
            .optional()?
            .unwrap_or(-1);
        Ok(expiry)
    }

    /// Set the message expiry setting for a group.
    pub fn set_group_expiry(&self, group_id: Uuid, seconds: i64) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE groups SET message_expiry_seconds = ?2 WHERE id = ?1",
            params![group_id.to_string(), seconds],
        )?;
        Ok(())
    }

    /// Get the visibility setting for a group.
    pub fn get_group_visibility(&self, group_id: Uuid) -> Result<i32> {
        let conn = self.lock_conn();
        let visibility: i32 = conn
            .prepare("SELECT visibility FROM groups WHERE id = ?1")?
            .query_row(params![group_id.to_string()], |row| row.get(0))
            .optional()?
            .unwrap_or(1);
        Ok(visibility)
    }

    /// Set the visibility setting for a group.
    pub fn set_group_visibility(&self, group_id: Uuid, visibility: i32) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE groups SET visibility = ?2 WHERE id = ?1",
            params![group_id.to_string(), visibility],
        )?;
        Ok(())
    }

    /// List all public groups, optionally filtered by name pattern.
    pub fn list_public_groups(
        &self,
        pattern: Option<&str>,
    ) -> Result<Vec<crate::db::PublicGroupRow>> {
        let conn = self.lock_conn();
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match pattern {
            Some(pat) => (
                "SELECT g.id, g.group_name, g.alias, COUNT(gm.user_id) as member_count
                 FROM groups g
                 LEFT JOIN group_members gm ON g.id = gm.group_id
                 WHERE g.visibility = 2 AND g.group_name LIKE ?1
                 GROUP BY g.id
                 ORDER BY g.group_name ASC"
                    .to_string(),
                vec![Box::new(format!("%{pat}%")) as Box<dyn rusqlite::types::ToSql>],
            ),
            None => (
                "SELECT g.id, g.group_name, g.alias, COUNT(gm.user_id) as member_count
                 FROM groups g
                 LEFT JOIN group_members gm ON g.id = gm.group_id
                 WHERE g.visibility = 2
                 GROUP BY g.id
                 ORDER BY g.group_name ASC"
                    .to_string(),
                vec![],
            ),
        };
        let mut stmt = conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                let id_str: String = row.get(0)?;
                Ok((id_str, row.get(1)?, row.get(2)?, row.get::<_, i64>(3)?))
            })?
            .collect::<std::result::Result<Vec<(String, String, Option<String>, i64)>, _>>()?;
        let mut result = Vec::with_capacity(rows.len());
        for (id_str, group_name, alias, member_count) in rows {
            let group_id = Uuid::parse_str(&id_str)
                .map_err(|e| Error::Internal(format!("invalid group UUID: {e}")))?;
            result.push(crate::db::PublicGroupRow {
                group_id,
                group_name,
                alias,
                member_count: member_count as u32,
            });
        }
        Ok(result)
    }

    /// Delete a group. CASCADE handles all dependent rows (group_members,
    /// pending_welcomes, messages, message_fetch_watermarks, etc.).
    pub fn delete_group(&self, group_id: Uuid) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "DELETE FROM groups WHERE id = ?1",
            params![group_id.to_string()],
        )?;
        Ok(())
    }

    /// Check whether a user is an admin of a group.
    pub fn is_group_admin(&self, group_id: Uuid, user_id: Uuid) -> Result<bool> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT 1 FROM group_members WHERE group_id = ?1 AND user_id = ?2 AND role = 'admin'",
        )?;
        let exists: Option<i64> = stmt
            .query_row(params![group_id.to_string(), user_id.to_string()], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(exists.is_some())
    }

    /// Promote a member to admin.
    pub fn promote_member(&self, group_id: Uuid, user_id: Uuid) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE group_members SET role = 'admin' WHERE group_id = ?1 AND user_id = ?2",
            params![group_id.to_string(), user_id.to_string()],
        )?;
        Ok(())
    }

    /// Demote an admin to regular member.
    pub fn demote_member(&self, group_id: Uuid, user_id: Uuid) -> Result<()> {
        let conn = self.lock_conn();
        conn.execute(
            "UPDATE group_members SET role = 'member' WHERE group_id = ?1 AND user_id = ?2",
            params![group_id.to_string(), user_id.to_string()],
        )?;
        Ok(())
    }

    /// Count the number of admins in a group.
    pub fn count_group_admins(&self, group_id: Uuid) -> Result<i64> {
        let conn = self.lock_conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM group_members WHERE group_id = ?1 AND role = 'admin'",
            params![group_id.to_string()],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// List admin members of a group.
    pub fn get_group_admins(&self, group_id: Uuid) -> Result<Vec<UserInfo>> {
        let conn = self.lock_conn();
        let mut stmt = conn.prepare(
            "SELECT u.id, u.username, u.alias, u.signing_key_fingerprint
             FROM users u
             JOIN group_members gm ON u.id = gm.user_id
             WHERE gm.group_id = ?1 AND gm.role = 'admin'",
        )?;
        let rows = stmt
            .query_map(params![group_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                Ok((id_str, row.get(1)?, row.get(2)?, row.get(3)?))
            })?
            .collect::<std::result::Result<Vec<(String, String, Option<String>, Option<String>)>, _>>()?;
        let mut result = Vec::with_capacity(rows.len());
        for (id_str, username, alias, signing_key_fingerprint) in rows {
            let user_id = Uuid::parse_str(&id_str)
                .map_err(|e| Error::Internal(format!("invalid user UUID: {e}")))?;
            result.push(UserInfo {
                user_id,
                username,
                alias,
                signing_key_fingerprint,
            });
        }
        Ok(result)
    }
}
