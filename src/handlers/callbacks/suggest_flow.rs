//! Callback handlers for the button-driven repository suggestion flow.

use crate::db::Database;
use crate::dialogue::state::{DialogueState, SuggestState};
use crate::dialogue::suggest::validate_new_suggestion;
use crate::dialogue::BotDialogue;
use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardMarkup, ParseMode};

/// Creates the pending suggestion and notifies admins.
#[allow(clippy::too_many_arguments)]
async fn file_suggestion(
    bot: &Bot,
    q: &CallbackQuery,
    dialogue: &BotDialogue,
    db: &Database,
    user_id: i64,
    username: Option<String>,
    owner: &str,
    name: &str,
    tags: &[String],
    description: Option<&str>,
) -> Result<()> {
    let tags_str = if tags.is_empty() {
        None
    } else {
        Some(
            tags.iter()
                .map(|t| format!("#{}", t))
                .collect::<Vec<_>>()
                .join(" "),
        )
    };

    let sugg_id = db
        .suggestions()
        .create_suggestion(
            user_id,
            username.as_deref(),
            owner,
            name,
            tags_str.as_deref(),
            description,
        )
        .await?;

    dialogue.exit().await?;

    if let Some(msg) = &q.message {
        let _ = bot
            .edit_message_text(
                msg.chat().id,
                msg.id(),
                format!(
                    "✅ Заявка #{} на отслеживание репозитория <b>{}/{}</b> отправлена на рассмотрение администраторам!",
                    sugg_id, owner, name
                ),
            )
            .parse_mode(ParseMode::Html)
            .await;
    }

    // Notify all admins in DM
    let kb = InlineKeyboardMarkup::new(vec![vec![
        teloxide::types::InlineKeyboardButton::callback(
            "✅ Одобрить",
            format!("suggest_approve:{}", sugg_id),
        ),
        teloxide::types::InlineKeyboardButton::callback(
            "❌ Отклонить",
            format!("suggest_reject:{}", sugg_id),
        ),
    ]]);

    let user_info = match &username {
        Some(u) => format!("@{} (<code>{}</code>)", u, user_id),
        None => format!("<code>{}</code>", user_id),
    };

    let desc_note = match description {
        Some(d) => format!(
            "\n📝 Описание: <i>{}</i>",
            html_escape::encode_text(d.trim())
        ),
        None => String::new(),
    };

    let admin_notice = format!(
        "💡 <b>Новая заявка на отслеживание репозитория #{}</b>\n\
        📦 Репозиторий: <code>{}/{}</code>{}\n\
        🏷️ Предложенные теги: <code>{}</code>\n\
        👤 Автор: {}",
        sugg_id,
        owner,
        name,
        desc_note,
        tags_str.as_deref().unwrap_or("нет"),
        user_info
    );

    super::notify_admins(bot, db, admin_notice, Some(kb), None).await?;

    Ok(())
}

pub async fn handle_suggest_flow(
    bot: &Bot,
    q: &CallbackQuery,
    action: &str,
    dialogue: &BotDialogue,
    db: &Database,
    user_id: i64,
    username: Option<String>,
) -> Result<()> {
    let cur_state = dialogue.get().await?;

    if action == "cancel" {
        dialogue.exit().await?;
        if let Some(msg) = &q.message {
            let _ = bot
                .edit_message_text(msg.chat().id, msg.id(), "❌ Предложение отменено.")
                .await;
        }
        return Ok(());
    }

    let Some(DialogueState::Suggest(state)) = cur_state else {
        return Ok(());
    };

    match (action, state) {
        ("send", SuggestState::Confirm { data }) => {
            // Re-validate right before filing (state may be stale).
            let repo = crate::config::RepoConfig {
                owner: data.owner.clone(),
                name: data.name.clone(),
            };
            let chat_id = q
                .message
                .as_ref()
                .map(|m| m.chat().id)
                .unwrap_or(ChatId(user_id));
            if !validate_new_suggestion(bot, chat_id, user_id, &repo, db).await? {
                dialogue.exit().await?;
                return Ok(());
            }

            file_suggestion(
                bot,
                q,
                dialogue,
                db,
                user_id,
                username,
                &data.owner,
                &data.name,
                &data.tags,
                data.description.as_deref(),
            )
            .await
        }
        ("desc", SuggestState::Confirm { data }) => {
            let cur_desc = data.description.clone().unwrap_or_default();
            dialogue
                .update(DialogueState::Suggest(SuggestState::WaitingDescription { data }))
                .await?;
            if let Some(msg) = &q.message {
                let _ = bot
                    .edit_message_text(
                        msg.chat().id,
                        msg.id(),
                        format!(
                            "📝 Введите описание репозитория (что это за инструмент) — оно будет показано администраторам и применится при одобрении.\n\
                            Текущее: <i>{}</i>",
                            html_escape::encode_text(cur_desc.trim())
                        ),
                    )
                    .reply_markup(crate::dialogue::suggest::skip_or_cancel_keyboard())
                    .parse_mode(ParseMode::Html)
                    .await;
            }
            Ok(())
        }
        ("tags", SuggestState::Confirm { data }) => {
            dialogue
                .update(DialogueState::Suggest(SuggestState::WaitingTags { data }))
                .await?;
            if let Some(msg) = &q.message {
                let _ = bot
                    .edit_message_text(
                        msg.chat().id,
                        msg.id(),
                        "🏷 Введите теги через пробел (например: <code>vpn android</code>):",
                    )
                    .reply_markup(crate::dialogue::suggest::skip_or_cancel_keyboard())
                    .parse_mode(ParseMode::Html)
                    .await;
            }
            Ok(())
        }
        ("skip", SuggestState::WaitingDescription { data }) => {
            dialogue
                .update(DialogueState::Suggest(SuggestState::Confirm {
                    data: data.clone(),
                }))
                .await?;
            if let Some(msg) = &q.message {
                let _ = bot.delete_message(msg.chat().id, msg.id()).await;
                crate::dialogue::suggest::send_suggest_confirm(bot, msg.chat().id, &data).await?;
            }
            Ok(())
        }
        ("skip", SuggestState::WaitingTags { data }) => {
            dialogue
                .update(DialogueState::Suggest(SuggestState::Confirm {
                    data: data.clone(),
                }))
                .await?;
            if let Some(msg) = &q.message {
                let _ = bot.delete_message(msg.chat().id, msg.id()).await;
                crate::dialogue::suggest::send_suggest_confirm(bot, msg.chat().id, &data).await?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
