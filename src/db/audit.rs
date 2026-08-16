//! Audit trail of administrative actions.

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct AdminActionRecord {
    #[allow(dead_code)]
    pub id: i64,
    pub admin_id: i64,
    pub action: String,
    pub target: String,
    pub created_at: String,
}

pub struct AuditRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> AuditRepo<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn log_action(&self, admin_id: i64, action: &str, target: &str) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO admin_actions (admin_id, action, target, created_at)
            VALUES (?, ?, ?, datetime('now'))
            "#,
        )
        .bind(admin_id)
        .bind(action)
        .bind(target)
        .execute(self.pool)
        .await
        .context("Failed to write admin action to audit log")?;
        Ok(())
    }

    /// Newest actions first, `limit` rows (paginated by offset).
    pub async fn recent_actions(&self, limit: i64, offset: i64) -> Result<Vec<AdminActionRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, admin_id, action, target, created_at
            FROM admin_actions
            ORDER BY id DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(self.pool)
        .await
        .context("Failed to list admin actions")?;

        Ok(rows
            .into_iter()
            .map(|r| AdminActionRecord {
                id: r.get("id"),
                admin_id: r.get("admin_id"),
                action: r.get("action"),
                target: r.get("target"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    pub async fn count_actions(&self) -> Result<i64> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM admin_actions")
            .fetch_one(self.pool)
            .await
            .context("Failed to count admin actions")?;
        Ok(count)
    }
}
