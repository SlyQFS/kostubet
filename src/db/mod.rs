//! SQLite database abstraction, schema initialization, and repository accessors.
//!
//! Configures connection pooling with WAL journal mode, busy timeouts, and
//! automated idempotent migrations.

pub mod admins;
pub mod custom_apps;
pub mod suggestions;
pub mod tags;
pub mod tools;

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;
use std::time::Duration;
use tracing::info;

pub use admins::AdminsRepo;
pub use custom_apps::CustomAppsRepo;
pub use suggestions::SuggestionsRepo;
#[allow(unused_imports)]
pub use tags::{ItemType, TagsRepo};
pub use tools::ToolsRepo;

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
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .with_context(|| format!("Failed to connect to SQLite database at {}", db_path))?;

        let db = Self { pool };
        db.init_schema().await?;
        Ok(db)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn admins(&self) -> AdminsRepo<'_> {
        AdminsRepo::new(&self.pool)
    }

    pub fn tools(&self) -> ToolsRepo<'_> {
        ToolsRepo::new(&self.pool)
    }

    pub fn tags(&self) -> TagsRepo<'_> {
        TagsRepo::new(&self.pool)
    }

    pub fn suggestions(&self) -> SuggestionsRepo<'_> {
        SuggestionsRepo::new(&self.pool)
    }

    pub fn custom_apps(&self) -> CustomAppsRepo<'_> {
        CustomAppsRepo::new(&self.pool)
    }

    pub async fn init_schema(&self) -> Result<()> {
        sqlx::query("PRAGMA foreign_keys = ON;")
            .execute(&self.pool)
            .await
            .context("Failed to enable SQLite foreign keys pragma")?;

        sqlx::query(
            r#"
            -- Admins
            CREATE TABLE IF NOT EXISTS admins (
                telegram_id INTEGER PRIMARY KEY,
                username    TEXT,
                is_owner    INTEGER NOT NULL DEFAULT 0,
                added_by    INTEGER,
                added_at    TEXT NOT NULL
            );

            -- Tracked GitHub tools
            CREATE TABLE IF NOT EXISTS tracked_tools (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                owner        TEXT NOT NULL,
                repo         TEXT NOT NULL,
                last_release TEXT,
                etag         TEXT,
                added_by     INTEGER NOT NULL DEFAULT 0,
                added_at     TEXT NOT NULL,
                UNIQUE(owner, repo)
            );

            -- Canonical tags
            CREATE TABLE IF NOT EXISTS tags (
                id   INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT UNIQUE NOT NULL
            );

            -- Polymorphic tags for tools and custom apps
            CREATE TABLE IF NOT EXISTS item_tags (
                item_type TEXT NOT NULL CHECK (item_type IN ('tool', 'custom_app')),
                item_id   INTEGER NOT NULL,
                tag_id    INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
                PRIMARY KEY (item_type, item_id, tag_id)
            );

            -- Repository suggestions
            CREATE TABLE IF NOT EXISTS suggestions (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id       INTEGER NOT NULL,
                username      TEXT,
                owner         TEXT NOT NULL,
                repo          TEXT NOT NULL,
                proposed_tags TEXT,
                status        TEXT NOT NULL DEFAULT 'pending',
                reviewed_by   INTEGER,
                reviewed_at   TEXT,
                created_at    TEXT NOT NULL
            );

            -- Custom applications
            CREATE TABLE IF NOT EXISTS custom_apps (
                id                 INTEGER PRIMARY KEY AUTOINCREMENT,
                slug               TEXT UNIQUE NOT NULL,
                name               TEXT NOT NULL,
                current_version_id INTEGER,
                created_by         INTEGER NOT NULL,
                created_at         TEXT NOT NULL
            );

            -- Custom application versions
            CREATE TABLE IF NOT EXISTS custom_app_versions (
                id                    INTEGER PRIMARY KEY AUTOINCREMENT,
                app_id                INTEGER NOT NULL REFERENCES custom_apps(id) ON DELETE CASCADE,
                version               TEXT NOT NULL,
                title                 TEXT,
                changelog             TEXT,
                diff_url              TEXT,
                cover_image_file_id   TEXT,
                submitted_by          INTEGER NOT NULL,
                status                TEXT NOT NULL DEFAULT 'pending',
                reviewed_by           INTEGER,
                reviewed_at           TEXT,
                published_message_id  INTEGER,
                created_at            TEXT NOT NULL
            );

            -- APK files for custom app versions
            CREATE TABLE IF NOT EXISTS custom_app_apk_files (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                version_id     INTEGER NOT NULL REFERENCES custom_app_versions(id) ON DELETE CASCADE,
                variant_label  TEXT NOT NULL,
                file_id        TEXT NOT NULL,
                file_unique_id TEXT NOT NULL,
                file_name      TEXT,
                file_size      INTEGER
            );

            -- Persistent FSM dialogue state
            CREATE TABLE IF NOT EXISTS dialogue_state (
                chat_id INTEGER PRIMARY KEY,
                state   TEXT NOT NULL
            );
            "#,
        )
        .execute(&self.pool)
        .await
        .context("Failed to execute database schema initialization")?;

        // Idempotent check: ensure etag column exists in tracked_tools for pre-existing tables
        let has_etag: Option<i64> = sqlx::query_scalar(
            "SELECT COUNT(1) FROM pragma_table_info('tracked_tools') WHERE name = 'etag';",
        )
        .fetch_optional(&self.pool)
        .await
        .unwrap_or(None);

        if has_etag == Some(0) {
            let _ = sqlx::query("ALTER TABLE tracked_tools ADD COLUMN etag TEXT;")
                .execute(&self.pool)
                .await;
        }

        // Non-destructive migration from legacy tracked_repos table if present
        let has_legacy_table: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='tracked_repos';",
        )
        .fetch_optional(&self.pool)
        .await
        .unwrap_or(None);

        if has_legacy_table.is_some() {
            info!("Legacy tracked_repos table detected. Migrating existing records into tracked_tools non-destructively...");
            let _ = sqlx::query(
                r#"
                INSERT OR IGNORE INTO tracked_tools (owner, repo, last_release, etag, added_by, added_at)
                SELECT owner, name, COALESCE(last_seen_id, last_seen_sha), etag, 0, COALESCE(updated_at, datetime('now'))
                FROM tracked_repos;
                "#,
            )
            .execute(&self.pool)
            .await;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_db_full_flow() -> Result<()> {
        let db = Database::new(":memory:").await?;

        // 1. Admins
        db.admins().seed_admins(&[100, 200]).await?;
        assert!(db.admins().is_admin(100).await?);
        assert!(db.admins().is_owner(100).await?);
        assert!(db.admins().is_admin(200).await?);
        assert!(!db.admins().is_owner(200).await?);
        assert!(!db.admins().is_admin(300).await?);

        // Add new admin
        assert!(
            db.admins()
                .add_admin(300, Some("user3"), Some(100), false)
                .await?
        );
        assert!(db.admins().is_admin(300).await?);

        // Remove admin (non-owner)
        assert!(db.admins().remove_admin(300).await?);
        assert!(!db.admins().is_admin(300).await?);

        // Cannot remove owner
        assert!(db.admins().remove_admin(100).await.is_err());

        // 2. Tools, Tags & ETag
        let tool_id = db.tools().add_tool("tokio-rs", "tokio", 100).await?;
        let tag_id = db.tags().get_or_create_tag("async").await?;
        db.tags()
            .attach_tag(ItemType::Tool, tool_id, tag_id)
            .await?;

        let tags = db.tags().get_tags_for_item(ItemType::Tool, tool_id).await?;
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].name, "async");

        db.tools()
            .update_last_release_and_etag(tool_id, Some("v1.40.0"), Some("W/\"etag_123\""))
            .await?;
        let tool = db.tools().get_tool("tokio-rs", "tokio").await?.unwrap();
        assert_eq!(tool.last_release.as_deref(), Some("v1.40.0"));
        assert_eq!(tool.etag.as_deref(), Some("W/\"etag_123\""));

        // 3. Suggestions
        let sugg_id = db
            .suggestions()
            .create_suggestion(
                400,
                Some("suggester"),
                "rust-lang",
                "rust",
                Some("compiler, language"),
            )
            .await?;
        assert_eq!(db.suggestions().count_pending_for_user(400).await?, 1);

        let updated = db
            .suggestions()
            .set_suggestion_status_if_pending(sugg_id, "approved", 100)
            .await?;
        assert!(updated);
        assert_eq!(db.suggestions().count_pending_for_user(400).await?, 0);

        // 4. Custom apps
        let app = db
            .custom_apps()
            .get_or_create_app("test-app", "Test App", 400)
            .await?;
        let ver_id = db
            .custom_apps()
            .create_version(
                app.id,
                "1.0.0",
                Some("v1.0.0 Title"),
                Some("Changelog"),
                None,
                None,
                400,
            )
            .await?;
        db.custom_apps()
            .add_apk_file(
                ver_id,
                "universal",
                "file_123",
                "uniq_123",
                Some("app-universal.apk"),
                Some(1024),
            )
            .await?;

        let pending_vers = db.custom_apps().get_pending_versions().await?;
        assert_eq!(pending_vers.len(), 1);

        let ver_approved = db
            .custom_apps()
            .set_version_status_if_pending(ver_id, "approved", 100, None)
            .await?;
        assert!(ver_approved);
        db.custom_apps()
            .set_app_current_version(app.id, ver_id)
            .await?;
        db.custom_apps()
            .set_published_message_id(ver_id, 777)
            .await?;

        let current = db.custom_apps().get_current_version(app.id).await?.unwrap();
        assert_eq!(current.version, "1.0.0");
        assert_eq!(current.published_message_id, Some(777));

        Ok(())
    }

    #[tokio::test]
    async fn test_legacy_migration_safety() -> Result<()> {
        let pool = SqlitePool::connect(":memory:").await?;

        // Create legacy table mimicking old database with etag
        sqlx::query(
            r#"
            CREATE TABLE tracked_repos (
                owner TEXT NOT NULL,
                name TEXT NOT NULL,
                last_seen_id TEXT,
                last_seen_sha TEXT,
                etag TEXT,
                updated_at TEXT,
                PRIMARY KEY (owner, name)
            );
            INSERT INTO tracked_repos (owner, name, last_seen_id, last_seen_sha, etag, updated_at)
            VALUES ('tokio-rs', 'tokio', '356880872', NULL, 'W/"etag123"', '2026-08-12 17:11:10');
            "#,
        )
        .execute(&pool)
        .await?;

        let db = Database { pool: pool.clone() };
        db.init_schema().await?;

        // Verify tracked_tools has the migrated repo with etag
        let tool = db
            .tools()
            .get_tool("tokio-rs", "tokio")
            .await?
            .expect("Should be migrated");
        assert_eq!(tool.owner, "tokio-rs");
        assert_eq!(tool.repo, "tokio");
        assert_eq!(tool.last_release.as_deref(), Some("356880872"));
        assert_eq!(tool.etag.as_deref(), Some("W/\"etag123\""));

        // Verify legacy table is still intact and not dropped
        let legacy_count: i64 = sqlx::query_scalar("SELECT COUNT(1) FROM tracked_repos")
            .fetch_one(&pool)
            .await?;
        assert_eq!(legacy_count, 1);

        Ok(())
    }
}
