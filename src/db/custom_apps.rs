//! Custom applications, versions, and APK file records repository.
//!
//! Manages custom app lifecycle (submission, admin review, status transition,
//! published post message tracking, and multi-architecture APK assets).

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub struct CustomAppRecord {
    pub id: i64,
    pub slug: String,
    pub name: String,
    /// Optional app-level description (what the app is; changelog is per version).
    pub description: Option<String>,
    #[allow(dead_code)]
    pub current_version_id: Option<i64>,
    #[allow(dead_code)]
    pub created_by: i64,
    #[allow(dead_code)]
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CustomAppVersionRecord {
    pub id: i64,
    pub app_id: i64,
    pub version: String,
    pub title: Option<String>,
    pub changelog: Option<String>,
    pub diff_url: Option<String>,
    pub cover_image_file_id: Option<String>,
    pub submitted_by: i64,
    pub status: String,
    #[allow(dead_code)]
    pub reviewed_by: Option<i64>,
    #[allow(dead_code)]
    pub reviewed_at: Option<String>,
    #[allow(dead_code)]
    pub published_message_id: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct CustomAppApkFileRecord {
    #[allow(dead_code)]
    pub id: i64,
    #[allow(dead_code)]
    pub version_id: i64,
    pub variant_label: String,
    pub file_id: String,
    #[allow(dead_code)]
    pub file_unique_id: String,
    pub file_name: Option<String>,
    #[allow(dead_code)]
    pub file_size: Option<i64>,
}

pub struct CustomAppsRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> CustomAppsRepo<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_or_create_app(
        &self,
        slug: &str,
        name: &str,
        description: Option<&str>,
        created_by: i64,
    ) -> Result<CustomAppRecord> {
        let clean_slug = slug.trim().to_lowercase();
        if let Some(existing) = self.get_app_by_slug(&clean_slug).await? {
            return Ok(existing);
        }

        let res = sqlx::query(
            r#"
            INSERT INTO custom_apps (slug, name, description, created_by, created_at)
            VALUES (?, ?, ?, ?, datetime('now'))
            "#,
        )
        .bind(&clean_slug)
        .bind(name)
        .bind(description)
        .bind(created_by)
        .execute(self.pool)
        .await
        .context("Failed to insert custom app")?;

        let id = res.last_insert_rowid();
        self.get_app_by_id(id)
            .await?
            .context("Failed to fetch newly created custom app")
    }

    /// Sets or clears (None) the app-level description.
    pub async fn set_app_description(&self, app_id: i64, description: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE custom_apps SET description = ? WHERE id = ?")
            .bind(description)
            .bind(app_id)
            .execute(self.pool)
            .await
            .context("Failed to update custom app description")?;
        Ok(())
    }

    pub async fn get_app_by_slug(&self, slug: &str) -> Result<Option<CustomAppRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, slug, name, description, current_version_id, created_by, created_at
            FROM custom_apps
            WHERE slug = ?
            "#,
        )
        .bind(slug.trim().to_lowercase())
        .fetch_optional(self.pool)
        .await
        .context("Failed to query custom app by slug")?;

        Ok(row.map(|r| CustomAppRecord {
            id: r.get("id"),
            slug: r.get("slug"),
            name: r.get("name"),
            description: r.get("description"),
            current_version_id: r.get("current_version_id"),
            created_by: r.get("created_by"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn get_app_by_id(&self, id: i64) -> Result<Option<CustomAppRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, slug, name, description, current_version_id, created_by, created_at
            FROM custom_apps
            WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_optional(self.pool)
        .await
        .context("Failed to query custom app by id")?;

        Ok(row.map(|r| CustomAppRecord {
            id: r.get("id"),
            slug: r.get("slug"),
            name: r.get("name"),
            description: r.get("description"),
            current_version_id: r.get("current_version_id"),
            created_by: r.get("created_by"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn list_approved_apps(&self) -> Result<Vec<CustomAppRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, slug, name, description, current_version_id, created_by, created_at
            FROM custom_apps
            WHERE current_version_id IS NOT NULL
            ORDER BY name ASC
            "#,
        )
        .fetch_all(self.pool)
        .await
        .context("Failed to list approved custom apps")?;

        Ok(rows
            .into_iter()
            .map(|r| CustomAppRecord {
                id: r.get("id"),
                slug: r.get("slug"),
                name: r.get("name"),
                description: r.get("description"),
                current_version_id: r.get("current_version_id"),
                created_by: r.get("created_by"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_version(
        &self,
        app_id: i64,
        version: &str,
        title: Option<&str>,
        changelog: Option<&str>,
        diff_url: Option<&str>,
        cover_image_file_id: Option<&str>,
        submitted_by: i64,
    ) -> Result<i64> {
        let res = sqlx::query(
            r#"
            INSERT INTO custom_app_versions (
                app_id, version, title, changelog, diff_url,
                cover_image_file_id, submitted_by, status, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', datetime('now'))
            "#,
        )
        .bind(app_id)
        .bind(version)
        .bind(title)
        .bind(changelog)
        .bind(diff_url)
        .bind(cover_image_file_id)
        .bind(submitted_by)
        .execute(self.pool)
        .await
        .context("Failed to create custom app version")?;

        Ok(res.last_insert_rowid())
    }

    pub async fn update_version_fields(
        &self,
        version_id: i64,
        title: Option<&str>,
        changelog: Option<&str>,
        diff_url: Option<&str>,
        cover_image_file_id: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE custom_app_versions
            SET title = ?, changelog = ?, diff_url = ?, cover_image_file_id = ?
            WHERE id = ?
            "#,
        )
        .bind(title)
        .bind(changelog)
        .bind(diff_url)
        .bind(cover_image_file_id)
        .bind(version_id)
        .execute(self.pool)
        .await
        .context("Failed to update custom app version fields")?;

        Ok(())
    }

    pub async fn add_apk_file(
        &self,
        version_id: i64,
        variant_label: &str,
        file_id: &str,
        file_unique_id: &str,
        file_name: Option<&str>,
        file_size: Option<i64>,
    ) -> Result<i64> {
        let res = sqlx::query(
            r#"
            INSERT INTO custom_app_apk_files (
                version_id, variant_label, file_id, file_unique_id, file_name, file_size
            )
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(version_id)
        .bind(variant_label)
        .bind(file_id)
        .bind(file_unique_id)
        .bind(file_name)
        .bind(file_size)
        .execute(self.pool)
        .await
        .context("Failed to insert APK file")?;

        Ok(res.last_insert_rowid())
    }

    pub async fn get_version(&self, version_id: i64) -> Result<Option<CustomAppVersionRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, app_id, version, title, changelog, diff_url,
                   cover_image_file_id, submitted_by, status, reviewed_by,
                   reviewed_at, published_message_id, created_at
            FROM custom_app_versions
            WHERE id = ?
            "#,
        )
        .bind(version_id)
        .fetch_optional(self.pool)
        .await
        .context("Failed to get custom app version by id")?;

        Ok(row.map(|r| CustomAppVersionRecord {
            id: r.get("id"),
            app_id: r.get("app_id"),
            version: r.get("version"),
            title: r.get("title"),
            changelog: r.get("changelog"),
            diff_url: r.get("diff_url"),
            cover_image_file_id: r.get("cover_image_file_id"),
            submitted_by: r.get("submitted_by"),
            status: r.get("status"),
            reviewed_by: r.get("reviewed_by"),
            reviewed_at: r.get("reviewed_at"),
            published_message_id: r.get("published_message_id"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn get_current_version(&self, app_id: i64) -> Result<Option<CustomAppVersionRecord>> {
        let row = sqlx::query(
            r#"
            SELECT v.id, v.app_id, v.version, v.title, v.changelog, v.diff_url,
                   v.cover_image_file_id, v.submitted_by, v.status, v.reviewed_by,
                   v.reviewed_at, v.published_message_id, v.created_at
            FROM custom_app_versions v
            JOIN custom_apps a ON a.current_version_id = v.id
            WHERE a.id = ?
            "#,
        )
        .bind(app_id)
        .fetch_optional(self.pool)
        .await
        .context("Failed to get current version for custom app")?;

        Ok(row.map(|r| CustomAppVersionRecord {
            id: r.get("id"),
            app_id: r.get("app_id"),
            version: r.get("version"),
            title: r.get("title"),
            changelog: r.get("changelog"),
            diff_url: r.get("diff_url"),
            cover_image_file_id: r.get("cover_image_file_id"),
            submitted_by: r.get("submitted_by"),
            status: r.get("status"),
            reviewed_by: r.get("reviewed_by"),
            reviewed_at: r.get("reviewed_at"),
            published_message_id: r.get("published_message_id"),
            created_at: r.get("created_at"),
        }))
    }

    /// Latest version of any status (incl. pending) — used for the
    /// "author differs from previous version" warning so that a pending
    /// claim by another user is also visible to admins.
    pub async fn get_latest_version_any_status(
        &self,
        app_id: i64,
    ) -> Result<Option<CustomAppVersionRecord>> {
        let row = sqlx::query(
            r#"
            SELECT id, app_id, version, title, changelog, diff_url,
                   cover_image_file_id, submitted_by, status, reviewed_by,
                   reviewed_at, published_message_id, created_at
            FROM custom_app_versions
            WHERE app_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(app_id)
        .fetch_optional(self.pool)
        .await
        .context("Failed to get latest version of any status")?;

        Ok(row.map(|r| CustomAppVersionRecord {
            id: r.get("id"),
            app_id: r.get("app_id"),
            version: r.get("version"),
            title: r.get("title"),
            changelog: r.get("changelog"),
            diff_url: r.get("diff_url"),
            cover_image_file_id: r.get("cover_image_file_id"),
            submitted_by: r.get("submitted_by"),
            status: r.get("status"),
            reviewed_by: r.get("reviewed_by"),
            reviewed_at: r.get("reviewed_at"),
            published_message_id: r.get("published_message_id"),
            created_at: r.get("created_at"),
        }))
    }

    pub async fn get_apk_files(&self, version_id: i64) -> Result<Vec<CustomAppApkFileRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, version_id, variant_label, file_id, file_unique_id, file_name, file_size
            FROM custom_app_apk_files
            WHERE version_id = ?
            ORDER BY id ASC
            "#,
        )
        .bind(version_id)
        .fetch_all(self.pool)
        .await
        .context("Failed to list APK files for version")?;

        Ok(rows
            .into_iter()
            .map(|r| CustomAppApkFileRecord {
                id: r.get("id"),
                version_id: r.get("version_id"),
                variant_label: r.get("variant_label"),
                file_id: r.get("file_id"),
                file_unique_id: r.get("file_unique_id"),
                file_name: r.get("file_name"),
                file_size: r.get("file_size"),
            })
            .collect())
    }

    pub async fn get_pending_versions(
        &self,
    ) -> Result<Vec<(CustomAppVersionRecord, CustomAppRecord)>> {
        let rows = sqlx::query(
            r#"
            SELECT v.id, v.app_id, v.version, v.title, v.changelog, v.diff_url,
                   v.cover_image_file_id, v.submitted_by, v.status, v.reviewed_by,
                   v.reviewed_at, v.published_message_id, v.created_at,
                   a.slug, a.name, a.description, a.current_version_id, a.created_by, a.created_at as app_created_at
            FROM custom_app_versions v
            JOIN custom_apps a ON v.app_id = a.id
            WHERE v.status = 'pending'
            ORDER BY v.created_at ASC
            "#,
        )
        .fetch_all(self.pool)
        .await
        .context("Failed to list pending custom app versions")?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let ver = CustomAppVersionRecord {
                    id: r.get("id"),
                    app_id: r.get("app_id"),
                    version: r.get("version"),
                    title: r.get("title"),
                    changelog: r.get("changelog"),
                    diff_url: r.get("diff_url"),
                    cover_image_file_id: r.get("cover_image_file_id"),
                    submitted_by: r.get("submitted_by"),
                    status: r.get("status"),
                    reviewed_by: r.get("reviewed_by"),
                    reviewed_at: r.get("reviewed_at"),
                    published_message_id: r.get("published_message_id"),
                    created_at: r.get("created_at"),
                };
                let app = CustomAppRecord {
                    id: r.get("app_id"),
                    slug: r.get("slug"),
                    name: r.get("name"),
                    description: r.get("description"),
                    current_version_id: r.get("current_version_id"),
                    created_by: r.get("created_by"),
                    created_at: r.get("app_created_at"),
                };
                (ver, app)
            })
            .collect())
    }

    pub async fn count_pending_versions_for_user(&self, user_id: i64) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(1) FROM custom_app_versions WHERE submitted_by = ? AND status = 'pending'",
        )
        .bind(user_id)
        .fetch_one(self.pool)
        .await
        .context("Failed to count pending versions for user")?;

        Ok(count)
    }

    pub async fn get_user_versions(
        &self,
        user_id: i64,
    ) -> Result<Vec<(CustomAppVersionRecord, CustomAppRecord)>> {
        let rows = sqlx::query(
            r#"
            SELECT v.id, v.app_id, v.version, v.title, v.changelog, v.diff_url,
                   v.cover_image_file_id, v.submitted_by, v.status, v.reviewed_by,
                   v.reviewed_at, v.published_message_id, v.created_at,
                   a.slug, a.name, a.description, a.current_version_id, a.created_by, a.created_at as app_created_at
            FROM custom_app_versions v
            JOIN custom_apps a ON v.app_id = a.id
            WHERE v.submitted_by = ?
            ORDER BY v.created_at DESC
            LIMIT 10
            "#,
        )
        .bind(user_id)
        .fetch_all(self.pool)
        .await
        .context("Failed to get user versions")?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let ver = CustomAppVersionRecord {
                    id: r.get("id"),
                    app_id: r.get("app_id"),
                    version: r.get("version"),
                    title: r.get("title"),
                    changelog: r.get("changelog"),
                    diff_url: r.get("diff_url"),
                    cover_image_file_id: r.get("cover_image_file_id"),
                    submitted_by: r.get("submitted_by"),
                    status: r.get("status"),
                    reviewed_by: r.get("reviewed_by"),
                    reviewed_at: r.get("reviewed_at"),
                    published_message_id: r.get("published_message_id"),
                    created_at: r.get("created_at"),
                };
                let app = CustomAppRecord {
                    id: r.get("app_id"),
                    slug: r.get("slug"),
                    name: r.get("name"),
                    description: r.get("description"),
                    current_version_id: r.get("current_version_id"),
                    created_by: r.get("created_by"),
                    created_at: r.get("app_created_at"),
                };
                (ver, app)
            })
            .collect())
    }

    pub async fn set_version_status_if_pending(
        &self,
        version_id: i64,
        status: &str,
        reviewed_by: i64,
        published_message_id: Option<i64>,
    ) -> Result<bool> {
        let res = sqlx::query(
            r#"
            UPDATE custom_app_versions
            SET status = ?, reviewed_by = ?, reviewed_at = datetime('now'), published_message_id = ?
            WHERE id = ? AND status = 'pending'
            "#,
        )
        .bind(status)
        .bind(reviewed_by)
        .bind(published_message_id)
        .bind(version_id)
        .execute(self.pool)
        .await
        .context("Failed to update custom app version status atomically")?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn set_published_message_id(&self, version_id: i64, message_id: i64) -> Result<()> {
        sqlx::query("UPDATE custom_app_versions SET published_message_id = ? WHERE id = ?")
            .bind(message_id)
            .bind(version_id)
            .execute(self.pool)
            .await
            .context("Failed to set published_message_id")?;
        Ok(())
    }

    /// Rolls an approved version back to `pending` (used when publishing fails
    /// after a successful moderation claim, so the request returns to the queue).
    pub async fn reset_version_to_pending(&self, version_id: i64) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE custom_app_versions
            SET status = 'pending', reviewed_by = NULL, reviewed_at = NULL
            WHERE id = ?
            "#,
        )
        .bind(version_id)
        .execute(self.pool)
        .await
        .context("Failed to reset version to pending")?;
        Ok(())
    }

    pub async fn set_app_current_version(&self, app_id: i64, version_id: i64) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE custom_apps
            SET current_version_id = ?
            WHERE id = ?
            "#,
        )
        .bind(version_id)
        .bind(app_id)
        .execute(self.pool)
        .await
        .context("Failed to set custom app current version")?;

        Ok(())
    }

    /// All published Telegram message ids across the app's versions
    /// (used to remove channel posts when deleting an app).
    pub async fn get_published_message_ids_for_app(&self, app_id: i64) -> Result<Vec<i64>> {
        let ids: Vec<i64> = sqlx::query_scalar(
            r#"
            SELECT published_message_id FROM custom_app_versions
            WHERE app_id = ? AND published_message_id IS NOT NULL
            "#,
        )
        .bind(app_id)
        .fetch_all(self.pool)
        .await
        .context("Failed to list published message ids for app")?;
        Ok(ids)
    }

    /// Permanently deletes an app with all its versions and APK file records
    /// (FK cascades) and detaches its tags. Returns false when the app
    /// did not exist.
    pub async fn delete_app(&self, app_id: i64) -> Result<bool> {
        sqlx::query("DELETE FROM item_tags WHERE item_type = 'custom_app' AND item_id = ?")
            .bind(app_id)
            .execute(self.pool)
            .await
            .context("Failed to detach app tags")?;

        let res = sqlx::query("DELETE FROM custom_apps WHERE id = ?")
            .bind(app_id)
            .execute(self.pool)
            .await
            .context("Failed to delete custom app")?;

        Ok(res.rows_affected() > 0)
    }
}
