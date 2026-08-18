//! Admin dialogue for button-driven management flows.
//!
//! Handles text input steps started from the admin panel inline keyboards:
//! adding a tracked repository (link -> mode -> optional description ->
//! optional tags), editing repo/app descriptions, and creating tags
//! (globally or attached to a specific item).

use crate::config::RepoConfig;
use crate::db::tags::ItemType;
use crate::db::Database;
use crate::dialogue::state::{AdminState, DialogueState};
use crate::dialogue::{validate_description, BotDialogue};
use anyhow::Result;
use html_escape::encode_text;
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
                    "❌ Отправьте ссылку вида: <code>https://github.com/owner/example</code> или <code>owner/example</code>",
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
                    "📦 Репозиторий <b>{}</b> найден.\n\nЧто сделать с текущим релизом?",
                    repo_label
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(InlineKeyboardMarkup::new(vec![
                vec![InlineKeyboardButton::callback(
                    "📣 Опубликовать релиз",
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

        AdminState::RepoDesc {
            owner,
            name,
            silent,
        } => {
            let mut description: Option<String> = None;
            if text != "/skip" && !text.is_empty() {
                if let Err(err) = validate_description(&text) {
                    bot.send_message(
                        chat_id,
                        format!("{}\n\nПовторите ввод или отправьте <code>/skip</code>.", err),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }
                description = Some(text.clone());
            }

            dialogue
                .update(DialogueState::Admin(Box::new(AdminState::RepoTags {
                    owner,
                    name,
                    silent,
                    description,
                })))
                .await?;

            let mode_note = if silent {
                "🔇 Текущий релиз будет пропущен."
            } else {
                "📣 Текущий релиз будет опубликован при первом опросе."
            };
            bot.send_message(
                chat_id,
                format!(
                    "🏷 Введите теги для репозитория через пробел (например: <code>rust network</code>)\n\
                    или отправьте <code>/done</code>, чтобы добавить без тегов.\n\n{}",
                    mode_note
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }

        AdminState::RepoTags {
            owner,
            name,
            silent,
            description,
        } => {
            let tags: Vec<String> = if text == "/done" || text == "/skip" {
                Vec::new()
            } else {
                text.split_whitespace()
                    .map(|t| t.trim_start_matches('#').to_string())
                    .filter(|t| !t.is_empty())
                    .collect()
            };

            let tool_id = db
                .tools()
                .add_tool(&owner, &name, sender_id, description.as_deref(), None)
                .await?;
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

            let desc_note = match &description {
                Some(d) => format!("\n📝 Описание: <i>{}</i>", encode_text(d)),
                None => String::new(),
            };

            let _ = db
                .audit()
                .log_action(sender_id, "добавил репозиторий", &format!("{}/{}", owner, name))
                .await;

            dialogue.exit().await?;

            bot.send_message(
                chat_id,
                format!(
                    "✅ Репозиторий <b>{}/{}</b> добавлен в отслеживание!\nТеги: <code>{}</code>{}{}",
                    owner, name, tags_str, desc_note, silent_note
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }

        AdminState::RepoDescription { tool_id } => {
            let Some(tool) = db.tools().get_tool_by_id(tool_id).await? else {
                dialogue.exit().await?;
                bot.send_message(chat_id, "⚠️ Репозиторий не найден (возможно, уже удален).")
                    .await?;
                return Ok(());
            };

            if text == "/skip" {
                dialogue.exit().await?;
                bot.send_message(chat_id, "ℹ️ Описание не изменено.").await?;
                return Ok(());
            }

            if text == "/clear" {
                db.tools().set_tool_description(tool_id, None).await?;
                let _ = db
                    .audit()
                    .log_action(
                        sender_id,
                        "удалил описание репозитория",
                        &tool.full_name(),
                    )
                    .await;
                dialogue.exit().await?;
                bot.send_message(
                    chat_id,
                    format!(
                        "✅ Описание репозитория <b>{}</b> удалено.",
                        encode_text(&tool.full_name())
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            if text.is_empty() {
                bot.send_message(
                    chat_id,
                    "❌ Описание не может быть пустым. Отправьте текст, <code>/skip</code> (оставить как есть) или <code>/clear</code> (удалить).",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            if let Err(err) = validate_description(&text) {
                bot.send_message(chat_id, err).await?;
                return Ok(());
            }

            db.tools()
                .set_tool_description(tool_id, Some(&text))
                .await?;
            let _ = db
                .audit()
                .log_action(sender_id, "изменил описание репозитория", &tool.full_name())
                .await;
            dialogue.exit().await?;
            let kb = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("📢 Опубликовать", format!("adm:repopost:{}", tool_id)),
                    InlineKeyboardButton::callback("📦 Репозиторий", format!("adm:repo:{}", tool_id)),
                ],
            ]);
            bot.send_message(
                chat_id,
                format!(
                    "✅ Описание репозитория <b>{}</b> обновлено.",
                    encode_text(&tool.full_name())
                ),
            )
            .reply_markup(kb)
            .parse_mode(ParseMode::Html)
            .await?;
        }

        AdminState::AppDescription { app_id } => {
            let Some(app) = db.custom_apps().get_app_by_id(app_id).await? else {
                dialogue.exit().await?;
                bot.send_message(chat_id, "⚠️ Приложение не найдено (возможно, удалено).")
                    .await?;
                return Ok(());
            };

            if text == "/skip" {
                dialogue.exit().await?;
                bot.send_message(chat_id, "ℹ️ Описание не изменено.").await?;
                return Ok(());
            }

            if text == "/clear" {
                db.custom_apps().set_app_description(app_id, None).await?;
                let _ = db
                    .audit()
                    .log_action(sender_id, "удалил описание приложения", &app.name)
                    .await;
                dialogue.exit().await?;
                let kb = InlineKeyboardMarkup::new(vec![
                    vec![
                        InlineKeyboardButton::callback("📢 Опубликовать", format!("adm:apppost:{}", app_id)),
                        InlineKeyboardButton::callback("📱 К приложениям", "adm:apps:0"),
                    ],
                ]);
                bot.send_message(
                    chat_id,
                    format!(
                        "✅ Описание приложения <b>{}</b> удалено.",
                        encode_text(&app.name)
                    ),
                )
                .reply_markup(kb)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            if text.is_empty() {
                bot.send_message(
                    chat_id,
                    "❌ Описание не может быть пустым. Отправьте текст, <code>/skip</code> (оставить как есть) или <code>/clear</code> (удалить).",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            if let Err(err) = validate_description(&text) {
                bot.send_message(chat_id, err).await?;
                return Ok(());
            }

            db.custom_apps()
                .set_app_description(app_id, Some(&text))
                .await?;
            let _ = db
                .audit()
                .log_action(sender_id, "изменил описание приложения", &app.name)
                .await;
            dialogue.exit().await?;
            let kb = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("📢 Опубликовать", format!("adm:apppost:{}", app_id)),
                    InlineKeyboardButton::callback("📱 К приложениям", "adm:apps:0"),
                ],
            ]);
            bot.send_message(
                chat_id,
                format!(
                    "✅ Описание приложения <b>{}</b> обновлено.",
                    encode_text(&app.name)
                ),
            )
            .reply_markup(kb)
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
