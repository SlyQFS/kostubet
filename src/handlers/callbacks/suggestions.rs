//! Callback query handlers for repository suggestion moderation.

use crate::db::tags::ItemType;
use crate::db::Database;
use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::{ChatId, ParseMode};
use tracing::warn;

pub async fn handle_suggestion_approve(
    bot: &Bot,
    q: &CallbackQuery,
    sugg_id: i64,
    db: &Database,
    user_id: i64,
    admin_name: &str,
) -> Result<()> {
    let Some(sugg) = db.suggestions().get_suggestion(sugg_id).await? else {
        return Ok(());
    };

    // The repository may have been deleted/renamed since the suggestion was
    // filed — reject instead of creating a dead tracker.
    if let Some(client) = crate::services::github::global() {
        match client.repo_exists(&sugg.owner, &sugg.repo).await {
            Ok(true) => {}
            Ok(false) => {
                let _ = db
                    .suggestions()
                    .set_suggestion_status_if_pending(sugg_id, "rejected", user_id)
                    .await;
                let _ = db
                    .audit()
                    .log_action(
                        user_id,
                        "отклонил предложку (репозиторий недоступен)",
                        &sugg.full_name(),
                    )
                    .await;
                if let Some(msg) = &q.message {
                    let _ = bot
                        .edit_message_text(
                            msg.chat().id,
                            msg.id(),
                            format!(
                                "❌ Репозиторий <b>{}</b> не найден на GitHub (404). Заявка #{} отклонена.",
                                sugg.full_name(),
                                sugg_id
                            ),
                        )
                        .parse_mode(ParseMode::Html)
                        .await;
                }
                return Ok(());
            }
            Err(e) => {
                warn!("Не удалось проверить существование {}: {:?}", sugg.full_name(), e);
            }
        }
    }

    // Add to tracked tools FIRST: on failure the claim is rolled back so the
    // suggestion returns to the queue instead of hanging "approved but untracked".
    let add_result = db
        .tools()
        .add_tool(
            &sugg.owner,
            &sugg.repo,
            user_id,
            sugg.proposed_description.as_deref(),
            sugg.username.as_deref(),
        )
        .await;

    let tool_id = match add_result {
        Ok(id) => id,
        Err(e) => {
            let _ = db.suggestions().reset_suggestion_to_pending(sugg_id).await;
            if let Some(msg) = &q.message {
                let _ = bot
                    .edit_message_text(
                        msg.chat().id,
                        msg.id(),
                        format!(
                            "❌ Ошибка добавления репозитория {} в отслеживание: {}.\nЗаявка #{} возвращена в очередь.",
                            sugg.full_name(),
                            e,
                            sugg_id
                        ),
                    )
                    .await;
            }
            return Ok(());
        }
    };

    let updated = db
        .suggestions()
        .set_suggestion_status_if_pending(sugg_id, "approved", user_id)
        .await?;

    if !updated {
        // Another admin processed this suggestion while we were adding the
        // tool; add_tool is get-or-create so the duplicate add is harmless.
        if let Some(msg) = &q.message {
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    format!(
                        "⚠️ Заявка #{} на <b>{}</b> уже была обработана другим администратором.",
                        sugg_id,
                        sugg.full_name()
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await;
        }
        return Ok(());
    }

    if let Some(ref tags_str) = sugg.proposed_tags {
        let tags: Vec<String> = tags_str
            .split(&[' ', ','][..])
            .map(|t| t.trim().trim_start_matches('#').to_string())
            .filter(|t| !t.is_empty())
            .collect();
        let _ = db
            .tags()
            .set_tags_for_item(ItemType::Tool, tool_id, &tags)
            .await;
    }

    let _ = db
        .audit()
        .log_action(user_id, "одобрил предложку", &sugg.full_name())
        .await;

    if let Some(msg) = &q.message {
        let _ = bot
            .edit_message_text(
                msg.chat().id,
                msg.id(),
                format!(
                    "✅ Заявка #{} на <b>{}</b> одобрена администратором {} и добавлена в отслеживание!",
                    sugg_id,
                    sugg.full_name(),
                    admin_name
                ),
            )
            .parse_mode(ParseMode::Html)
            .await;
    }

    // Notify author
    if let Err(e) = bot
        .send_message(
            ChatId(sugg.user_id),
            format!(
                "🎉 Ваша заявка на отслеживание репозитория <b>{}</b> была одобрена администратором!",
                sugg.full_name()
            ),
        )
        .parse_mode(ParseMode::Html)
        .await
    {
        warn!("Failed to notify suggestion author {}: {:?}", sugg.user_id, e);
    }

    Ok(())
}

pub async fn handle_suggestion_reject(
    bot: &Bot,
    q: &CallbackQuery,
    sugg_id: i64,
    db: &Database,
    user_id: i64,
    admin_name: &str,
) -> Result<()> {
    let Some(sugg) = db.suggestions().get_suggestion(sugg_id).await? else {
        return Ok(());
    };

    let updated = db
        .suggestions()
        .set_suggestion_status_if_pending(sugg_id, "rejected", user_id)
        .await?;

    if !updated {
        if let Some(msg) = &q.message {
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    format!(
                        "⚠️ Заявка #{} на <b>{}</b> уже была обработана другим администратором.",
                        sugg_id,
                        sugg.full_name()
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await;
        }
        return Ok(());
    }

    let _ = db
        .audit()
        .log_action(user_id, "отклонил предложку", &sugg.full_name())
        .await;

    if let Some(msg) = &q.message {
        let _ = bot
            .edit_message_text(
                msg.chat().id,
                msg.id(),
                format!(
                    "❌ Заявка #{} на <b>{}</b> отклонена администратором {}.",
                    sugg_id,
                    sugg.full_name(),
                    admin_name
                ),
            )
            .parse_mode(ParseMode::Html)
            .await;
    }

    // Notify author
    if let Err(e) = bot
        .send_message(
            ChatId(sugg.user_id),
            format!(
                "❌ Ваша заявка на отслеживание репозитория <b>{}</b> была отклонена администратором.",
                sugg.full_name()
            ),
        )
        .parse_mode(ParseMode::Html)
        .await
    {
        warn!("Failed to notify suggestion author {}: {:?}", sugg.user_id, e);
    }

    Ok(())
}
