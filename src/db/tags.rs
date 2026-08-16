//! Polymorphic tag storage and normalization repository.
//!
//! Provides tagging for both tracked GitHub tools and custom APK applications,
//! including hashtag normalization and many-to-many relationship management.

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    Tool,
    CustomApp,
}

impl ItemType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemType::Tool => "tool",
            ItemType::CustomApp => "custom_app",
        }
    }
}

impl fmt::Display for ItemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ItemType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tool" => Ok(ItemType::Tool),
            "custom_app" | "app" => Ok(ItemType::CustomApp),
            _ => anyhow::bail!("Invalid item type: {}", s),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TagRecord {
    #[allow(dead_code)]
    pub id: i64,
    pub name: String,
}

pub fn normalize_tag(name: &str) -> String {
    name.trim()
        .trim_start_matches('#')
        .replace(' ', "_")
        .to_lowercase()
}

pub struct TagsRepo<'a> {
    pool: &'a SqlitePool,
}

impl<'a> TagsRepo<'a> {
    pub fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_or_create_tag(&self, name: &str) -> Result<i64> {
        let clean = normalize_tag(name);
        if clean.is_empty() {
            anyhow::bail!("Tag name cannot be empty");
        }

        let existing: Option<i64> = sqlx::query_scalar("SELECT id FROM tags WHERE name = ?")
            .bind(&clean)
            .fetch_optional(self.pool)
            .await
            .context("Failed to check tag existence")?;

        if let Some(id) = existing {
            return Ok(id);
        }

        let res = sqlx::query("INSERT INTO tags (name) VALUES (?)")
            .bind(&clean)
            .execute(self.pool)
            .await
            .context("Failed to insert new tag")?;

        Ok(res.last_insert_rowid())
    }

    pub async fn remove_tag_by_id(&self, id: i64) -> Result<bool> {
        sqlx::query("DELETE FROM item_tags WHERE tag_id = ?")
            .bind(id)
            .execute(self.pool)
            .await
            .context("Failed to detach removed tag from items")?;

        let res = sqlx::query("DELETE FROM tags WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await
            .context("Failed to delete tag by id")?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn list_tags(&self) -> Result<Vec<TagRecord>> {
        let rows = sqlx::query("SELECT id, name FROM tags ORDER BY name ASC")
            .fetch_all(self.pool)
            .await
            .context("Failed to list tags")?;

        Ok(rows
            .into_iter()
            .map(|r| TagRecord {
                id: r.get("id"),
                name: r.get("name"),
            })
            .collect())
    }

    /// Lists all tags together with the number of items (tools + apps) using them.
    pub async fn list_tags_with_usage(&self) -> Result<Vec<(TagRecord, i64)>> {
        let rows = sqlx::query(
            r#"
            SELECT t.id, t.name, COUNT(it.item_id) AS usage_count
            FROM tags t
            LEFT JOIN item_tags it ON it.tag_id = t.id
            GROUP BY t.id, t.name
            ORDER BY t.name ASC
            "#,
        )
        .fetch_all(self.pool)
        .await
        .context("Failed to list tags with usage")?;

        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    TagRecord {
                        id: r.get("id"),
                        name: r.get("name"),
                    },
                    r.get("usage_count"),
                )
            })
            .collect())
    }

    /// Lists (item_type, item_id) pairs a tag is attached to.
    pub async fn list_items_for_tag(&self, tag_id: i64) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            r#"
            SELECT item_type, item_id
            FROM item_tags
            WHERE tag_id = ?
            ORDER BY item_type ASC, item_id ASC
            "#,
        )
        .bind(tag_id)
        .fetch_all(self.pool)
        .await
        .context("Failed to list items for tag")?;

        Ok(rows
            .into_iter()
            .map(|r| (r.get::<String, _>("item_type"), r.get::<i64, _>("item_id")))
            .collect())
    }

    pub async fn attach_tag(&self, item_type: ItemType, item_id: i64, tag_id: i64) -> Result<()> {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO item_tags (item_type, item_id, tag_id)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(item_type.as_str())
        .bind(item_id)
        .bind(tag_id)
        .execute(self.pool)
        .await
        .context("Failed to attach tag to item")?;

        Ok(())
    }

    pub async fn detach_tag(&self, item_type: ItemType, item_id: i64, tag_id: i64) -> Result<bool> {
        let res =
            sqlx::query("DELETE FROM item_tags WHERE item_type = ? AND item_id = ? AND tag_id = ?")
                .bind(item_type.as_str())
                .bind(item_id)
                .bind(tag_id)
                .execute(self.pool)
                .await
                .context("Failed to detach tag from item")?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn detach_tag_by_name(
        &self,
        item_type: ItemType,
        item_id: i64,
        tag_name: &str,
    ) -> Result<bool> {
        let clean = normalize_tag(tag_name);
        let tag_id: Option<i64> = sqlx::query_scalar("SELECT id FROM tags WHERE name = ?")
            .bind(&clean)
            .fetch_optional(self.pool)
            .await?;

        if let Some(tid) = tag_id {
            self.detach_tag(item_type, item_id, tid).await
        } else {
            Ok(false)
        }
    }

    pub async fn get_tags_for_item(
        &self,
        item_type: ItemType,
        item_id: i64,
    ) -> Result<Vec<TagRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT t.id, t.name
            FROM tags t
            JOIN item_tags it ON t.id = it.tag_id
            WHERE it.item_type = ? AND it.item_id = ?
            ORDER BY t.name ASC
            "#,
        )
        .bind(item_type.as_str())
        .bind(item_id)
        .fetch_all(self.pool)
        .await
        .context("Failed to fetch tags for item")?;

        Ok(rows
            .into_iter()
            .map(|r| TagRecord {
                id: r.get("id"),
                name: r.get("name"),
            })
            .collect())
    }

    pub async fn set_tags_for_item(
        &self,
        item_type: ItemType,
        item_id: i64,
        tag_names: &[String],
    ) -> Result<()> {
        // Clear existing tags
        sqlx::query("DELETE FROM item_tags WHERE item_type = ? AND item_id = ?")
            .bind(item_type.as_str())
            .bind(item_id)
            .execute(self.pool)
            .await
            .context("Failed to reset item tags")?;

        for tag_name in tag_names {
            let clean = normalize_tag(tag_name);
            if !clean.is_empty() {
                let tag_id = self.get_or_create_tag(&clean).await?;
                self.attach_tag(item_type, item_id, tag_id).await?;
            }
        }

        Ok(())
    }
}
