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

    let updated = db
        .suggestions()
        .set_suggestion_status_if_pending(sugg_id, "approved", user_id)
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

    // Add to tracked tools
    let tool_id = db
        .tools()
        .add_tool(&sugg.owner, &sugg.repo, user_id)
        .await?;
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
