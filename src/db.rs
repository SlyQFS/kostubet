use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct TrackedRepoRecord {
    pub owner: String,
    pub name: String,
    pub last_seen_id: Option<String>,
    pub last_seen_sha: Option<String>,
    pub etag: Option<String>,
    #[allow(dead_code)]
    pub updated_at: Option<String>,
}

impl TrackedRepoRecord {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(db_path: &str) -> Result<Self> {
        let connect_url = if db_path.starts_with("sqlite://") || db_path == ":memory:" {
            db_path.to_string()
        } else {
            format!("sqlite://{}", db_path)
        };

        let options = SqliteConnectOptions::from_str(&connect_url)
            .with_context(|| format!("Invalid SQLite connection string: {}", db_path))?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .with_context(|| format!("Failed to connect to SQLite database at {}", db_path))?;

        let db = Self { pool };
        db.init_schema().await?;
        Ok(db)
    }

    pub async fn init_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS tracked_repos (
                owner TEXT NOT NULL,
                name TEXT NOT NULL,
                last_seen_id TEXT,
                last_seen_sha TEXT,
                etag TEXT,
                updated_at TEXT,
                PRIMARY KEY (owner, name)
            );
            "#,
        )
        .execute(&self.pool)
        .await
        .context("Failed to execute database schema migration")?;

        Ok(())
    }

    pub async fn get_tracked_repos(&self) -> Result<Vec<TrackedRepoRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT owner, name, last_seen_id, last_seen_sha, etag, updated_at
            FROM tracked_repos
            ORDER BY owner ASC, name ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("Failed to fetch tracked repositories")?;

        Ok(rows
            .into_iter()
            .map(|r| TrackedRepoRecord {
                owner: r.get("owner"),
                name: r.get("name"),
                last_seen_id: r.get("last_seen_id"),
                last_seen_sha: r.get("last_seen_sha"),
                etag: r.get("etag"),
                updated_at: r.get("updated_at"),
            })
            .collect())
    }

    #[allow(dead_code)]
    pub async fn get_repo(&self, owner: &str, name: &str) -> Result<Option<TrackedRepoRecord>> {
        let row = sqlx::query(
            r#"
            SELECT owner, name, last_seen_id, last_seen_sha, etag, updated_at
            FROM tracked_repos
            WHERE owner = ? AND name = ?
            "#,
        )
        .bind(owner)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .context("Failed to query repo from database")?;

        Ok(row.map(|r| TrackedRepoRecord {
            owner: r.get("owner"),
            name: r.get("name"),
            last_seen_id: r.get("last_seen_id"),
            last_seen_sha: r.get("last_seen_sha"),
            etag: r.get("etag"),
            updated_at: r.get("updated_at"),
        }))
    }

    pub async fn add_repo(&self, owner: &str, name: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO tracked_repos (owner, name, updated_at)
            VALUES (?, ?, datetime('now'))
            "#,
        )
        .bind(owner)
        .bind(name)
        .execute(&self.pool)
        .await
        .context("Failed to insert repository")?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn remove_repo(&self, owner: &str, name: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM tracked_repos
            WHERE owner = ? AND name = ?
            "#,
        )
        .bind(owner)
        .bind(name)
        .execute(&self.pool)
        .await
        .context("Failed to delete repository")?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_seen(
        &self,
        owner: &str,
        name: &str,
        last_seen_id: Option<&str>,
        last_seen_sha: Option<&str>,
        etag: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO tracked_repos (owner, name, last_seen_id, last_seen_sha, etag, updated_at)
            VALUES (?, ?, ?, ?, ?, datetime('now'))
            ON CONFLICT(owner, name) DO UPDATE SET
                last_seen_id = COALESCE(excluded.last_seen_id, tracked_repos.last_seen_id),
                last_seen_sha = COALESCE(excluded.last_seen_sha, tracked_repos.last_seen_sha),
                etag = COALESCE(excluded.etag, tracked_repos.etag),
                updated_at = datetime('now')
            "#,
        )
        .bind(owner)
        .bind(name)
        .bind(last_seen_id)
        .bind(last_seen_sha)
        .bind(etag)
        .execute(&self.pool)
        .await
        .context("Failed to mark repo update as seen")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_db_operations() -> Result<()> {
        let db = Database::new(":memory:").await?;

        // Add repo
        let added = db.add_repo("tokio-rs", "tokio").await?;
        assert!(added);

        // Add duplicate
        let added_again = db.add_repo("tokio-rs", "tokio").await?;
        assert!(!added_again);

        // Get tracked
        let repos = db.get_tracked_repos().await?;
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].full_name(), "tokio-rs/tokio");

        // Mark seen
        db.mark_seen("tokio-rs", "tokio", Some("rel_123"), Some("sha_456"), Some("W/\"etag123\"")).await?;
        let repo = db.get_repo("tokio-rs", "tokio").await?.unwrap();
        assert_eq!(repo.last_seen_id.as_deref(), Some("rel_123"));
        assert_eq!(repo.last_seen_sha.as_deref(), Some("sha_456"));
        assert_eq!(repo.etag.as_deref(), Some("W/\"etag123\""));

        // Remove repo
        let removed = db.remove_repo("tokio-rs", "tokio").await?;
        assert!(removed);
        let repos_after = db.get_tracked_repos().await?;
        assert_eq!(repos_after.len(), 0);

        Ok(())
    }
}
