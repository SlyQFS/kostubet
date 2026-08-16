//! Bot administrators and ownership repository.
//!
//! Handles admin authorization, owner privileges, admin promotion/demotion,
//! and initial seeding from config files/environment variables.

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct AdminRecord {
    pub telegram_id: i64,
    pub username: Option<String>,
    pub is_owner: bool,
    #[allow(dead_code)]
    pub added_by: Option<i64>,
    #[allow(dead_code)]
    pub added_at: String,
}

impl AdminRecord {
    pub fn display_name(&self) -> String {
        match &self.username {
            Some(u) => format!("@{} (<code>{}</code>)", u, self.telegram_id),
            None => format!("<code>{}</code>", self.telegram_id),
        }
    }
}

pub struct AdminsRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> AdminsRepo<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn is_admin(&self, telegram_id: i64) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM admins WHERE telegram_id = ?")
            .bind(telegram_id)
            .fetch_one(self.pool)
            .await
            .context("Failed to check if user is admin")?;

        Ok(count > 0)
    }

    pub async fn is_owner(&self, telegram_id: i64) -> Result<bool> {
        let is_owner: Option<i64> =
            sqlx::query_scalar("SELECT is_owner FROM admins WHERE telegram_id = ?")
                .bind(telegram_id)
                .fetch_optional(self.pool)
                .await
                .context("Failed to check if user is owner")?;

        Ok(is_owner.unwrap_or(0) == 1)
    }

    pub async fn add_admin(
        &self,
        telegram_id: i64,
        username: Option<&str>,
        added_by: Option<i64>,
        is_owner: bool,
    ) -> Result<bool> {
        let res = sqlx::query(
            r#"
            INSERT INTO admins (telegram_id, username, is_owner, added_by, added_at)
            VALUES (?, ?, ?, ?, datetime('now'))
            ON CONFLICT(telegram_id) DO UPDATE SET
                username = COALESCE(excluded.username, admins.username),
                is_owner = MAX(admins.is_owner, excluded.is_owner)
            "#,
        )
        .bind(telegram_id)
        .bind(username)
        .bind(if is_owner { 1 } else { 0 })
        .bind(added_by)
        .execute(self.pool)
        .await
        .context("Failed to add admin")?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn remove_admin(&self, telegram_id: i64) -> Result<bool> {
        // Prevent deleting owners
        let is_owner = self.is_owner(telegram_id).await?;
        if is_owner {
            anyhow::bail!("Cannot remove bot owner from administrators.");
        }

        let res = sqlx::query("DELETE FROM admins WHERE telegram_id = ? AND is_owner = 0")
            .bind(telegram_id)
            .execute(self.pool)
            .await
            .context("Failed to delete admin")?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn list_admins(&self) -> Result<Vec<AdminRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT telegram_id, username, is_owner, added_by, added_at
            FROM admins
            ORDER BY is_owner DESC, added_at ASC
            "#,
        )
        .fetch_all(self.pool)
        .await
        .context("Failed to list admins")?;

        Ok(rows
            .into_iter()
            .map(|r| AdminRecord {
                telegram_id: r.get("telegram_id"),
                username: r.get("username"),
                is_owner: r.get::<i64, _>("is_owner") == 1,
                added_by: r.get("added_by"),
                added_at: r.get("added_at"),
            })
            .collect())
    }

    pub async fn seed_admins(&self, admin_ids: &[i64]) -> Result<()> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM admins")
            .fetch_one(self.pool)
            .await
            .context("Failed to count admins")?;

        // Only seed if admins table is completely empty
        if count == 0 && !admin_ids.is_empty() {
            for (idx, &admin_id) in admin_ids.iter().enumerate() {
                let is_owner = idx == 0;
                self.add_admin(admin_id, None, None, is_owner).await?;
            }
        }
        Ok(())
    }
}
