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
    /// Consecutive "repository not found" polls (dead-repo detection).
    pub fail_count: i64,
    #[allow(dead_code)]
    pub added_by: i64,
    #[allow(dead_code)]
    pub added_at: String,
    /// Optional human-readable description shown in release cards.
    pub description: Option<String>,
}

impl TrackedToolRecord {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    fn from_row(r: &sqlx::sqlite::SqliteRow) -> Self {
        Self {
            id: r.get("id"),
            owner: r.get("owner"),
            repo: r.get("repo"),
            last_release: r.get("last_release"),
            etag: r.get("etag"),
            fail_count: r.get::<i64, _>("fail_count"),
            added_by: r.get("added_by"),
            added_at: r.get("added_at"),
            description: r.get("description"),
        }
    }
}

pub struct ToolsRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ToolsRepo<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn add_tool(
        &self,
        owner: &str,
        repo: &str,
        added_by: i64,
        description: Option<&str>,
    ) -> Result<i64> {
        let existing = self.get_tool(owner, repo).await?;
        if let Some(tool) = existing {
            // The tool may already exist (e.g. tracked manually while a
            // suggestion was pending): apply the description, never clear it.
            if description.is_some() && tool.description.as_deref() != description {
                self.set_tool_description(tool.id, description).await?;
            }
            return Ok(tool.id);
        }

        let res = sqlx::query(
            r#"
            INSERT INTO tracked_tools (owner, repo, added_by, added_at, description)
            VALUES (?, ?, ?, datetime('now'), ?)
            "#,
        )
        .bind(owner)
        .bind(repo)
        .bind(added_by)
        .bind(description)
        .execute(self.pool)
        .await
        .context("Failed to insert tracked tool")?;

        Ok(res.last_insert_rowid())
    }

    /// Sets or clears (None) the optional description of a tracked tool.
    pub async fn set_tool_description(&self, id: i64, description: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE tracked_tools SET description = ? WHERE id = ?")
            .bind(description)
            .bind(id)
            .execute(self.pool)
            .await
            .context("Failed to update tracked tool description")?;
        Ok(())
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
            SELECT id, owner, repo, last_release, etag, fail_count, added_by, added_at, description
            FROM tracked_tools
            WHERE owner = ? AND repo = ?
            "#,
        )
        .bind(owner)
        .bind(repo)
        .fetch_optional(self.pool)
        .await
        .context("Failed to query tracked tool")?;

        Ok(row.map(|r| TrackedToolRecord::from_row(&r)))
    }

    pub async fn get_tool_by_id(&self, id: i64) -> Result<Option<TrackedToolRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, owner, repo, last_release, etag, fail_count, added_by, added_at, description
            FROM tracked_tools
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await
        .context("Failed to query tracked tool by id")?;

        Ok(row.map(|r| TrackedToolRecord::from_row(&r)))
    }

    pub async fn list_tools(&self) -> Result<Vec<TrackedToolRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, owner, repo, last_release, etag, fail_count, added_by, added_at, description
            FROM tracked_tools
            ORDER BY owner ASC, repo ASC
            "#,
        )
        .fetch_all(self.pool)
        .await
        .context("Failed to list tracked tools")?;

        Ok(rows.into_iter().map(|r| TrackedToolRecord::from_row(&r)).collect())
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
            SET last_release = ?, etag = ?, fail_count = 0
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

    /// Increments the consecutive-404 counter. Returns the new value.
    pub async fn bump_tool_failures(&self, id: i64) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            r#"
            UPDATE tracked_tools
            SET fail_count = fail_count + 1
            WHERE id = ?
            RETURNING fail_count
            "#,
        )
        .bind(id)
        .fetch_one(self.pool)
        .await
        .context("Failed to bump tool failure counter")?;
        Ok(count)
    }

    /// Resets the consecutive-404 counter after a successful check.
    pub async fn reset_tool_failures(&self, id: i64) -> Result<()> {
        sqlx::query("UPDATE tracked_tools SET fail_count = 0 WHERE id = ? AND fail_count > 0")
            .bind(id)
            .execute(self.pool)
            .await
            .context("Failed to reset tool failure counter")?;
        Ok(())
    }

    /// Repositories with recent not-found failures (for /debug diagnostics).
    pub async fn list_failing_tools(&self) -> Result<Vec<TrackedToolRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, owner, repo, last_release, etag, fail_count, added_by, added_at, description
            FROM tracked_tools
            WHERE fail_count > 0
            ORDER BY fail_count DESC, owner ASC
            "#,
        )
        .fetch_all(self.pool)
        .await
        .context("Failed to list failing tools")?;

        Ok(rows.into_iter().map(|r| TrackedToolRecord::from_row(&r)).collect())
    }
}
