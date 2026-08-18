//! SQLite dialogue storage engine for teloxide FSM.
//!
//! Stores serialized JSON state keyed by Telegram ChatId in the `dialogue_state` table.

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::SqlitePool;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use teloxide::dispatching::dialogue::Storage;
use teloxide::types::ChatId;

#[derive(Clone)]
pub struct SqliteDialogueStorage {
    pool: SqlitePool,
}

impl SqliteDialogueStorage {
    pub fn new(pool: SqlitePool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

#[derive(Debug)]
pub struct StorageError(pub anyhow::Error);

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for StorageError {}

impl<D> Storage<D> for SqliteDialogueStorage
where
    D: Default + Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
{
    type Error = StorageError;

    fn remove_dialogue(
        self: Arc<Self>,
        chat_id: ChatId,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send>>
    where
        D: Send + 'static,
    {
        Box::pin(async move {
            sqlx::query("DELETE FROM dialogue_state WHERE chat_id = ?")
                .bind(chat_id.0)
                .execute(&self.pool)
                .await
                .context("Failed to delete dialogue state")
                .map_err(StorageError)?;
            Ok(())
        })
    }

    fn update_dialogue(
        self: Arc<Self>,
        chat_id: ChatId,
        dialogue: D,
    ) -> Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send>>
    where
        D: Send + 'static,
    {
        Box::pin(async move {
            let json_str = serde_json::to_string(&dialogue)
                .context("Failed to serialize dialogue state")
                .map_err(StorageError)?;

            sqlx::query(
                r#"
                INSERT INTO dialogue_state (chat_id, state)
                VALUES (?, ?)
                ON CONFLICT(chat_id) DO UPDATE SET state = excluded.state
                "#,
            )
            .bind(chat_id.0)
            .bind(json_str)
            .execute(&self.pool)
            .await
            .context("Failed to update dialogue state")
            .map_err(StorageError)?;

            Ok(())
        })
    }

    fn get_dialogue(
        self: Arc<Self>,
        chat_id: ChatId,
    ) -> Pin<Box<dyn Future<Output = Result<Option<D>, Self::Error>> + Send>> {
        Box::pin(async move {
            let row: Option<String> =
                sqlx::query_scalar("SELECT state FROM dialogue_state WHERE chat_id = ?")
                    .bind(chat_id.0)
                    .fetch_optional(&self.pool)
                    .await
                    .context("Failed to get dialogue state")
                    .map_err(StorageError)?;

            match row {
                Some(json_str) => match serde_json::from_str::<D>(&json_str) {
                    Ok(val) => Ok(Some(val)),
                    Err(e) => {
                        // Corrupt/incompatible state must not vanish silently:
                        // log it loudly and drop the broken row so the user
                        // can start a fresh dialogue.
                        tracing::error!(
                            "Corrupt dialogue state for chat {}: {:?}. Resetting to default.",
                            chat_id.0,
                            e
                        );
                        let _ = sqlx::query("DELETE FROM dialogue_state WHERE chat_id = ?")
                            .bind(chat_id.0)
                            .execute(&self.pool)
                            .await;
                        Ok(Some(D::default()))
                    }
                },
                None => Ok(None),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::dialogue::state::{DialogueState, SubmitApkData, SubmitApkState};

    #[tokio::test]
    async fn test_sqlite_dialogue_storage() -> anyhow::Result<()> {
        let db = Database::new(":memory:").await?;
        let storage = SqliteDialogueStorage::new(db.pool().clone());

        let chat_id = ChatId(123456);

        // Initially none
        let state: Option<DialogueState> = storage.clone().get_dialogue(chat_id).await.unwrap();
        assert!(state.is_none());

        // Update dialogue
        let new_state = DialogueState::SubmitApk(SubmitApkState::WaitingName);
        storage
            .clone()
            .update_dialogue(chat_id, new_state.clone())
            .await
            .unwrap();

        let retrieved: Option<DialogueState> = storage.clone().get_dialogue(chat_id).await.unwrap();
        assert!(retrieved.is_some());
        assert!(matches!(
            retrieved.unwrap(),
            DialogueState::SubmitApk(SubmitApkState::WaitingName)
        ));

        // Update to nested struct
        let custom_state = DialogueState::SubmitApk(SubmitApkState::WaitingVersion {
            data: Box::new(SubmitApkData {
                is_new_app: true,
                app_id: None,
                slug: "test-app".to_string(),
                name: "Test App".to_string(),
                description: Some("Test description".to_string()),
                version: "1.0.0".to_string(),
                title: None,
                changelog: None,
                diff_url: None,
                cover_image_file_id: None,
                apk_files: Vec::new(),
                tags: vec!["tool".to_string()],
                submitted_by_username: Some("test_user".to_string()),
            }),
        });

        storage
            .clone()
            .update_dialogue(chat_id, custom_state)
            .await
            .unwrap();

        let retrieved_v2: Option<DialogueState> =
            storage.clone().get_dialogue(chat_id).await.unwrap();
        if let Some(DialogueState::SubmitApk(SubmitApkState::WaitingVersion { data })) =
            retrieved_v2
        {
            assert_eq!(data.name, "Test App");
            assert_eq!(data.tags, vec!["tool"]);
        } else {
            panic!("Expected SubmitApkState::WaitingVersion");
        }

        // Remove dialogue
        <SqliteDialogueStorage as Storage<DialogueState>>::remove_dialogue(
            storage.clone(),
            chat_id,
        )
        .await
        .unwrap();
        let after_remove: Option<DialogueState> =
            storage.clone().get_dialogue(chat_id).await.unwrap();
        assert!(after_remove.is_none());

        Ok(())
    }
}
