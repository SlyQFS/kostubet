//! Admin dialogue for editing pending APK application metadata.
//!
//! Allows admins to review and update changelogs, titles, GitHub diff URLs,
//! and category tags before final publication to channels.

use crate::db::Database;
use crate::dialogue::state::{DialogueState, EditApkState};
use crate::dialogue::BotDialogue;
use crate::services::render::{build_apk_post_data, render_post_text};
use anyhow::Result;
use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

#[tracing::instrument(skip(bot, dialogue, db))]
pub async fn handle_edit_message(
    bot: Bot,
    msg: Message,
    dialogue: BotDialogue,
    state: EditApkState,
    db: Database,
) -> Result<()> {
    let chat_id = msg.chat.id;
    let text = msg.text().unwrap_or("").trim();

    if text == "/cancel" {
        dialogue.exit().await?;
        bot.send_message(chat_id, "❌ Редактирование заявки отменено.")
            .await?;
        return Ok(());
    }

    match state {
        EditApkState::EditingTitle { mut data } => {
            if text != "/skip" && !text.is_empty() {
                data.title = Some(text.to_string());
            }

            let cur_changelog = data
                .changelog
                .clone()
                .unwrap_or_else(|| "не указан".to_string());
            dialogue
                .update(DialogueState::EditApk(EditApkState::EditingChangelog {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                format!(
                    "📝 <b>Текущий список изменений (Changelog):</b>\n<code>{}</code>\n\nВведите новый список изменений или отправьте <code>/skip</code>:",
                    encode_text(&cur_changelog)
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        EditApkState::EditingChangelog { mut data } => {
            if text != "/skip" && !text.is_empty() {
                data.changelog = Some(text.to_string());
            }

            let cur_diff = data
                .diff_url
                .clone()
                .unwrap_or_else(|| "не указана".to_string());
            dialogue
                .update(DialogueState::EditApk(EditApkState::EditingDiffUrl {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                format!(
                    "🔗 <b>Текущая ссылка на изменения (Diff URL):</b>\n<code>{}</code>\n\nВведите новую ссылку или отправьте <code>/skip</code>:",
                    encode_text(&cur_diff)
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        EditApkState::EditingDiffUrl { mut data } => {
            if text != "/skip" && !text.is_empty() {
                data.diff_url = Some(text.to_string());
            }

            let cur_tags = if data.tags.is_empty() {
                "нет тегов".to_string()
            } else {
                data.tags.join(", ")
            };

            dialogue
                .update(DialogueState::EditApk(EditApkState::EditingTags { data }))
                .await?;

            bot.send_message(
                chat_id,
                format!(
                    "🏷️ <b>Текущие теги:</b> <code>{}</code>\n\nВведите новые теги через пробел/запятую или отправьте <code>/skip</code>:",
                    encode_text(&cur_tags)
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        EditApkState::EditingTags { mut data } => {
            if text != "/skip" && !text.is_empty() {
                let tags: Vec<String> = text
                    .split(&[' ', ','][..])
                    .map(|t| t.trim().trim_start_matches('#').to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
                data.tags = tags;
            }

            let apk_files = db.custom_apps().get_apk_files(data.version_id).await?;
            let apk_tuples: Vec<(i64, String)> = apk_files
                .into_iter()
                .map(|f| (f.id, f.variant_label))
                .collect();

            let post = build_apk_post_data(
                &data.app_name,
                &data.version,
                data.changelog.clone(),
                data.diff_url.clone(),
                data.cover_image_file_id.clone(),
                data.tags.clone(),
                &apk_tuples,
            );

            let preview_text = render_post_text(&post);
            let confirm_kb = InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback(
                    "🚀 Опубликовать",
                    format!("edit_publish:{}", data.version_id),
                ),
                InlineKeyboardButton::callback("❌ Отмена", "edit_cancel"),
            ]]);

            let version_id = data.version_id;
            dialogue
                .update(DialogueState::EditApk(EditApkState::ConfirmEdit { data }))
                .await?;

            bot.send_message(
                chat_id,
                format!(
                    "👀 <b>Предпросмотр отредактированного релиза #{}</b>:\n\n{}\n\n━━━━━━━━━━━━━━━\nОпубликовать?",
                    version_id,
                    preview_text
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(confirm_kb)
            .await?;
        }
        EditApkState::ConfirmEdit { .. } => {
            bot.send_message(
                chat_id,
                "ℹ️ Подтвердите публикацию кнопкой <b>🚀 Опубликовать</b> или отмените кнопкой <b>❌ Отмена</b>.",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
    }

    Ok(())
}
