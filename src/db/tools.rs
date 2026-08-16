//! Tracked GitHub tools repository.
//!
//! Manages repository definitions, release cache IDs, and HTTP ETag headers
//! for conditional rate-efficient GitHub polling.

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct TrackedToolRecord {
    pub id: i64,
    pub owner: String,
    pub repo: String,
    pub last_release: Option<String>,
    pub etag: Option<String>,
    #[allow(dead_code)]
    pub added_by: i64,
    #[allow(dead_code)]
    pub added_at: String,
}

impl TrackedToolRecord {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

pub struct ToolsRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ToolsRepo<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn add_tool(&self, owner: &str, repo: &str, added_by: i64) -> Result<i64> {
        let existing = self.get_tool(owner, repo).await?;
        if let Some(tool) = existing {
            return Ok(tool.id);
        }

        let res = sqlx::query(
            r#"
            INSERT INTO tracked_tools (owner, repo, added_by, added_at)
            VALUES (?, ?, ?, datetime('now'))
            "#,
        )
        .bind(owner)
        .bind(repo)
        .bind(added_by)
        .execute(self.pool)
        .await
        .context("Failed to insert tracked tool")?;

        Ok(res.last_insert_rowid())
    }

    pub async fn remove_tool(&self, owner: &str, repo: &str) -> Result<bool> {
        if let Some(tool) = self.get_tool(owner, repo).await? {
            // Delete associated item_tags first
            let _ = sqlx::query("DELETE FROM item_tags WHERE item_type = 'tool' AND item_id = ?")
                .bind(tool.id)
                .execute(self.pool)
                .await;

            let res = sqlx::query("DELETE FROM tracked_tools WHERE id = ?")
                .bind(tool.id)
                .execute(self.pool)
                .await
                .context("Failed to delete tracked tool")?;

            Ok(res.rows_affected() > 0)
        } else {
            Ok(false)
        }
    }

    pub async fn get_tool(&self, owner: &str, repo: &str) -> Result<Option<TrackedToolRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, owner, repo, last_release, etag, added_by, added_at
            FROM tracked_tools
            WHERE owner = ? AND repo = ?
            "#,
        )
        .bind(owner)
        .bind(repo)
        .fetch_optional(self.pool)
        .await
        .context("Failed to query tracked tool")?;

        Ok(row.map(|r| TrackedToolRecord {
            id: r.get("id"),
            owner: r.get("owner"),
            repo: r.get("repo"),
            last_release: r.get("last_release"),
            etag: r.get("etag"),
            added_by: r.get("added_by"),
            added_at: r.get("added_at"),
        }))
    }

    pub async fn get_tool_by_id(&self, id: i64) -> Result<Option<TrackedToolRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, owner, repo, last_release, etag, added_by, added_at
            FROM tracked_tools
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await
        .context("Failed to query tracked tool by id")?;

        Ok(row.map(|r| TrackedToolRecord {
            id: r.get("id"),
            owner: r.get("owner"),
            repo: r.get("repo"),
            last_release: r.get("last_release"),
            etag: r.get("etag"),
            added_by: r.get("added_by"),
            added_at: r.get("added_at"),
        }))
    }

    pub async fn list_tools(&self) -> Result<Vec<TrackedToolRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, owner, repo, last_release, etag, added_by, added_at
            FROM tracked_tools
            ORDER BY owner ASC, repo ASC
            "#,
        )
        .fetch_all(self.pool)
        .await
        .context("Failed to list tracked tools")?;

        Ok(rows
            .into_iter()
            .map(|r| TrackedToolRecord {
                id: r.get("id"),
                owner: r.get("owner"),
                repo: r.get("repo"),
                last_release: r.get("last_release"),
                etag: r.get("etag"),
                added_by: r.get("added_by"),
                added_at: r.get("added_at"),
            })
            .collect())
    }

    pub async fn update_last_release_and_etag(
        &self,
        id: i64,
        last_release: Option<&str>,
        etag: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE tracked_tools
            SET last_release = ?, etag = ?
            WHERE id = ?
            "#,
        )
        .bind(last_release)
        .bind(etag)
        .bind(id)
        .execute(self.pool)
        .await
        .context("Failed to update last_release and etag")?;

        Ok(())
    }
}
