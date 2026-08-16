//! Admin dialogue for button-driven management flows.
//!
//! Handles text input steps started from the admin panel inline keyboards:
//! adding a tracked repository (link -> mode -> optional tags), and creating
//! tags (globally or attached to a specific item).

use crate::config::RepoConfig;
use crate::db::tags::ItemType;
use crate::db::Database;
use crate::dialogue::state::{AdminState, DialogueState};
use crate::dialogue::BotDialogue;
use anyhow::Result;
use std::str::FromStr;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle_admin_message(
    bot: Bot,
    msg: Message,
    dialogue: BotDialogue,
    state: AdminState,
    db: Database,
) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let text = msg.text().unwrap_or_default().trim().to_string();

    // Any dialogue may be aborted with /cancel
    if text == "/cancel" || text == "/start" {
        dialogue.exit().await?;
        if text == "/cancel" {
            bot.send_message(chat_id, "❌ Действие отменено.")
                .await?;
        }
        return Ok(());
    }

    match state {
        AdminState::RepoLink => {
            let Some(repo) = RepoConfig::parse_ref(&text) else {
                bot.send_message(
                    chat_id,
                    "❌ Не удалось распознать репозиторий. Отправьте ссылку вида:\n\
                    <code>https://github.com/owner/repo</code> или <code>owner/repo</code>\n\n\
                    Или отправьте <code>/cancel</code> для отмены.",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            };

            if let Some(existing) = db.tools().get_tool(&repo.owner, &repo.name).await? {
                bot.send_message(
                    chat_id,
                    format!(
                        "ℹ️ Репозиторий <b>{}</b> уже отслеживается (ID <code>{}</code>).",
                        existing.full_name(),
                        existing.id
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
                dialogue.exit().await?;
                return Ok(());
            }

            // Existence check: a typo must not create a dead 404-ing tracker.
            if let Some(client) = crate::services::github::global() {
                match client.repo_exists(&repo.owner, &repo.name).await {
                    Ok(true) => {}
                    Ok(false) => {
                        bot.send_message(
                            chat_id,
                            format!(
                                "❌ Репозиторий <b>{}</b> не найден на GitHub (404). Проверьте написание.",
                                repo.full_name()
                            ),
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                        dialogue.exit().await?;
                        return Ok(());
                    }
                    Err(e) => {
                        // GitHub unreachable: let the admin decide whether to proceed.
                        bot.send_message(
                            chat_id,
                            format!(
                                "⚠️ Не удалось проверить существование репозитория ({}).\nПродолжаю добавление.",
                                e
                            ),
                        )
                        .await?;
                    }
                }
            }

            let repo_label = repo.full_name();
            dialogue
                .update(DialogueState::Admin(Box::new(AdminState::RepoMode {
                    owner: repo.owner,
                    name: repo.name,
                })))
                .await?;

            bot.send_message(
                chat_id,
                format!(
                    "📦 Репозиторий <b>{}</b> найден.\n\nЧто сделать с его текущим релизом?",
                    repo_label
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "📣 Запостить текущий релиз",
                    "adm:trackmode:post",
                )],
                vec![InlineKeyboardButton::callback(
                    "🔇 Пропустить текущий релиз",
                    "adm:trackmode:silent",
                )],
                vec![InlineKeyboardButton::callback(
                    "❌ Отмена",
                    "submit_confirm:cancel",
                )],
            ]))
            .await?;
        }

        AdminState::RepoMode { .. } => {
            bot.send_message(
                chat_id,
                "ℹ️ Выберите действие кнопкой выше или отправьте <code>/cancel</code>.",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }

        AdminState::RepoTags {
            owner,
            name,
            silent,
        } => {
            let tags: Vec<String> = if text == "/done" || text == "/skip" {
                Vec::new()
            } else {
                text.split_whitespace()
                    .map(|t| t.trim_start_matches('#').to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            };

            let tool_id = db.tools().add_tool(&owner, &name, sender_id).await?;
            if !tags.is_empty() {
                let _ = db
                    .tags()
                    .set_tags_for_item(ItemType::Tool, tool_id, &tags)
                    .await;
            }

            // Silent mode: mark the current latest release as already seen so
            // the first poll does not post it.
            let mut silent_note = String::new();
            if silent {
                if let Some(client) = crate::services::github::global() {
                    if let Ok(Some((release_id, etag))) =
                        client.latest_release_brief(&owner, &name).await
                    {
                        let _ = db
                            .tools()
                            .update_last_release_and_etag(
                                tool_id,
                                Some(&release_id),
                                etag.as_deref(),
                            )
                            .await;
                        silent_note =
                            "\n🔇 Текущий релиз пропущен — будут публиковаться только новые."
                                .to_string();
                    }
                }
            }

            let tags_str = if tags.is_empty() {
                "без тегов".to_string()
            } else {
                tags.iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" ")
            };

            let _ = db
                .audit()
                .log_action(sender_id, "добавил репозиторий", &format!("{}/{}", owner, name))
                .await;

            dialogue.exit().await?;

            bot.send_message(
                chat_id,
                format!(
                    "✅ Репозиторий <b>{}/{}</b> добавлен в отслеживание!\nТеги: <code>{}</code>{}",
                    owner, name, tags_str, silent_note
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }

        AdminState::NewTag => {
            if text.is_empty() {
                bot.send_message(chat_id, "❌ Тег не может быть пустым. Отправьте название или <code>/cancel</code>.")
                    .parse_mode(ParseMode::Html)
                    .await?;
                return Ok(());
            }

            match db.tags().get_or_create_tag(&text).await {
                Ok(_) => {
                    bot.send_message(
                        chat_id,
                        format!("✅ Тег <b>#{}</b> создан.", crate::db::tags::normalize_tag(&text)),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                Err(e) => {
                    bot.send_message(chat_id, format!("❌ Ошибка создания тега: {}", e))
                        .await?;
                }
            }
            dialogue.exit().await?;
        }

        AdminState::ItemTag {
            item_type,
            item_id,
            item_label,
        } => {
            if text.is_empty() {
                bot.send_message(chat_id, "❌ Тег не может быть пустым. Отправьте название или <code>/cancel</code>.")
                    .parse_mode(ParseMode::Html)
                    .await?;
                return Ok(());
            }

            let it = ItemType::from_str(&item_type).unwrap_or(ItemType::Tool);
            let tag_name = crate::db::tags::normalize_tag(&text);

            if let Ok(tag_id) = db.tags().get_or_create_tag(&tag_name).await {
                let _ = db.tags().attach_tag(it, item_id, tag_id).await;
                bot.send_message(
                    chat_id,
                    format!("✅ Тег <b>#{}</b> добавлен к <b>{}</b>.", tag_name, item_label),
                )
                .parse_mode(ParseMode::Html)
                .await?;
            } else {
                bot.send_message(chat_id, "❌ Ошибка создания тега.").await?;
            }
            dialogue.exit().await?;
        }
    }

    Ok(())
}
