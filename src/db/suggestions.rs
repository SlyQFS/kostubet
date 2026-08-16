//! Repository suggestion moderation queue repository.
//!
//! Stores user-submitted GitHub repository suggestions and tracks their
//! approval/rejection lifecycle.

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct SuggestionRecord {
    pub id: i64,
    pub user_id: i64,
    pub username: Option<String>,
    pub owner: String,
    pub repo: String,
    pub proposed_tags: Option<String>,
    pub status: String,
    #[allow(dead_code)]
    pub reviewed_by: Option<i64>,
    #[allow(dead_code)]
    pub reviewed_at: Option<String>,
    pub created_at: String,
}

impl SuggestionRecord {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

pub struct SuggestionsRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SuggestionsRepo<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_suggestion(
        &self,
        user_id: i64,
        username: Option<&str>,
        owner: &str,
        repo: &str,
        proposed_tags: Option<&str>,
    ) -> Result<i64> {
        let res = sqlx::query(
            r#"
            INSERT INTO suggestions (user_id, username, owner, repo, proposed_tags, status, created_at)
            VALUES (?, ?, ?, ?, ?, 'pending', datetime('now'))
            "#,
        )
        .bind(user_id)
        .bind(username)
        .bind(owner)
        .bind(repo)
        .bind(proposed_tags)
        .execute(self.pool)
        .await
        .context("Failed to create suggestion")?;

        Ok(res.last_insert_rowid())
    }

    pub async fn count_pending_for_user(&self, user_id: i64) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM suggestions WHERE user_id = ? AND status = 'pending'",
        )
        .bind(user_id)
        .fetch_one(self.pool)
        .await
        .context("Failed to count pending suggestions for user")?;

        Ok(count)
    }

    pub async fn get_pending_suggestions(&self) -> Result<Vec<SuggestionRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, username, owner, repo, proposed_tags, status, reviewed_by, reviewed_at, created_at
            FROM suggestions
            WHERE status = 'pending'
            ORDER BY created_at ASC
            "#,
        )
        .fetch_all(self.pool)
        .await
        .context("Failed to list pending suggestions")?;

        Ok(rows
            .into_iter()
            .map(|r| SuggestionRecord {
                id: r.get("id"),
                user_id: r.get("user_id"),
                username: r.get("username"),
                owner: r.get("owner"),
                repo: r.get("repo"),
                proposed_tags: r.get("proposed_tags"),
                status: r.get("status"),
                reviewed_by: r.get("reviewed_by"),
                reviewed_at: r.get("reviewed_at"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Finds an existing pending suggestion for the exact repository.
    pub async fn find_pending_by_repo(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Option<SuggestionRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, username, owner, repo, proposed_tags, status, reviewed_by, reviewed_at, created_at
            FROM suggestions
            WHERE status = 'pending' AND owner = ? AND repo = ?
            LIMIT 1
            "#,
        )
        .bind(owner)
        .bind(repo)
        .fetch_optional(self.pool)
        .await
        .context("Failed to find pending suggestion by repo")?;

        Ok(row.map(|r| SuggestionRecord {
            id: r.get("id"),
            user_id: r.get("user_id"),
            username: r.get("username"),
            owner: r.get("owner"),
            repo: r.get("repo"),
            proposed_tags: r.get("proposed_tags"),
            status: r.get("status"),
            reviewed_by: r.get("reviewed_by"),
            reviewed_at: r.get("reviewed_at"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn get_suggestion(&self, id: i64) -> Result<Option<SuggestionRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, username, owner, repo, proposed_tags, status, reviewed_by, reviewed_at, created_at
            FROM suggestions
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await
        .context("Failed to get suggestion by id")?;

        Ok(row.map(|r| SuggestionRecord {
            id: r.get("id"),
            user_id: r.get("user_id"),
            username: r.get("username"),
            owner: r.get("owner"),
            repo: r.get("repo"),
            proposed_tags: r.get("proposed_tags"),
            status: r.get("status"),
            reviewed_by: r.get("reviewed_by"),
            reviewed_at: r.get("reviewed_at"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn get_user_suggestions(&self, user_id: i64) -> Result<Vec<SuggestionRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, user_id, username, owner, repo, proposed_tags, status, reviewed_by, reviewed_at, created_at
            FROM suggestions
            WHERE user_id = ?
            ORDER BY created_at DESC
            LIMIT 10
            "#,
        )
        .bind(user_id)
        .fetch_all(self.pool)
        .await
        .context("Failed to get user suggestions")?;

        Ok(rows
            .into_iter()
            .map(|r| SuggestionRecord {
                id: r.get("id"),
                user_id: r.get("user_id"),
                username: r.get("username"),
                owner: r.get("owner"),
                repo: r.get("repo"),
                proposed_tags: r.get("proposed_tags"),
                status: r.get("status"),
                reviewed_by: r.get("reviewed_by"),
                reviewed_at: r.get("reviewed_at"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Rolls an approved/rejected suggestion back to `pending` (used when
    /// post-claim steps fail so the request returns to the queue).
    pub async fn reset_suggestion_to_pending(&self, id: i64) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE suggestions
            SET status = 'pending', reviewed_by = NULL, reviewed_at = NULL
            WHERE id = ?
            "#,
        )
        .bind(id)
        .execute(self.pool)
        .await
        .context("Failed to reset suggestion to pending")?;
        Ok(())
    }

    pub async fn set_suggestion_status_if_pending(
        &self,
        id: i64,
        status: &str,
        reviewed_by: i64,
    ) -> Result<bool> {
        let res = sqlx::query(
            r#"
            UPDATE suggestions
            SET status = ?, reviewed_by = ?, reviewed_at = datetime('now')
            WHERE id = ? AND status = 'pending'
            "#,
        )
        .bind(status)
        .bind(reviewed_by)
        .bind(id)
        .execute(self.pool)
        .await
        .context("Failed to update suggestion status atomically")?;

        Ok(res.rows_affected() > 0)
    }
}
