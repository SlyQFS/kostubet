//! Callback query handlers for user APK submission wizard.

use crate::db::tags::ItemType;
use crate::db::Database;
use crate::dialogue::state::{DialogueState, SubmitApkData, SubmitApkState};
use crate::dialogue::BotDialogue;
use anyhow::Result;
use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle_submit_mode_new(
    bot: &Bot,
    q: &CallbackQuery,
    dialogue: &BotDialogue,
) -> Result<()> {
    dialogue
        .update(DialogueState::SubmitApk(SubmitApkState::WaitingName))
        .await?;

    if let Some(msg) = &q.message {
        bot.send_message(
            msg.chat().id,
            "📝 Введите название нового приложения (например, <i>V2RayNG</i>):",
        )
        .parse_mode(ParseMode::Html)
        .await?;
    }
    Ok(())
}

pub async fn handle_submit_mode_update(
    bot: &Bot,
    q: &CallbackQuery,
    dialogue: &BotDialogue,
    db: &Database,
) -> Result<()> {
    let approved_apps = db.custom_apps().list_approved_apps().await?;
    if approved_apps.is_empty() {
        dialogue
            .update(DialogueState::SubmitApk(SubmitApkState::WaitingName))
            .await?;

        if let Some(msg) = &q.message {
            bot.send_message(
                msg.chat().id,
                "📭 В каталоге пока нет опубликованных приложений.\n\n📝 Введите название нового приложения:",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        return Ok(());
    }

    let mut rows = Vec::new();
    for app in approved_apps {
        rows.push(vec![InlineKeyboardButton::callback(
            app.name.clone(),
            format!("submit_app:{}", app.slug),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "❌ Отмена",
        "submit_confirm:cancel",
    )]);

    dialogue
        .update(DialogueState::SubmitApk(
            SubmitApkState::ChoosingExistingApp,
        ))
        .await?;

    if let Some(msg) = &q.message {
        bot.send_message(msg.chat().id, "🔄 Выберите приложение для обновления:")
            .reply_markup(InlineKeyboardMarkup::new(rows))
            .await?;
    }
    Ok(())
}

pub async fn handle_submit_app_select(
    bot: &Bot,
    q: &CallbackQuery,
    slug: &str,
    dialogue: &BotDialogue,
    db: &Database,
) -> Result<()> {
    if let Some(app) = db.custom_apps().get_app_by_slug(slug).await? {
        let data = Box::new(SubmitApkData {
            is_new_app: false,
            app_id: Some(app.id),
            slug: app.slug,
            name: app.name,
            // App description is kept on the app; loaded for the card preview only.
            description: app.description.clone(),
            version: String::new(),
            title: None,
            changelog: None,
            diff_url: None,
            cover_image_file_id: None,
            apk_files: Vec::new(),
            tags: Vec::new(),
        });

        dialogue
            .update(DialogueState::SubmitApk(SubmitApkState::WaitingVersion {
                data,
            }))
            .await?;

        if let Some(msg) = &q.message {
            bot.send_message(
                msg.chat().id,
                "📦 Введите номер новой версии (например, <code>1.2.3</code>):",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
    }
    Ok(())
}

pub async fn handle_variant_select(
    bot: &Bot,
    q: &CallbackQuery,
    variant: &str,
    dialogue: &BotDialogue,
) -> Result<()> {
    let cur_state = dialogue.get().await?;
    if let Some(DialogueState::SubmitApk(SubmitApkState::ResolvingVariant {
        mut data,
        mut pending_file,
    })) = cur_state
    {
        pending_file.variant = variant.to_string();
        data.apk_files.push(pending_file);

        let count = data.apk_files.len();
        dialogue
            .update(DialogueState::SubmitApk(SubmitApkState::WaitingApkFiles {
                data,
            }))
            .await?;

        if let Some(msg) = &q.message {
            bot.send_message(
                msg.chat().id,
                format!(
                    "✅ Архитектура <b>{}</b> сохранена!\nВсего файлов: <b>{}</b>.\n\nОтправьте следующий .apk файл или команду <code>/done</code>.",
                    variant, count
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
    }
    Ok(())
}

pub async fn handle_submit_confirm_send(
    bot: &Bot,
    q: &CallbackQuery,
    dialogue: &BotDialogue,
    db: &Database,
    user_id: i64,
    username: Option<String>,
) -> Result<()> {
    let cur_state = dialogue.get().await?;
    if let Some(DialogueState::SubmitApk(SubmitApkState::Confirm { data })) = cur_state {
        let chat_id = q
            .message
            .as_ref()
            .map(|m| m.chat().id)
            .unwrap_or(ChatId(user_id));

        // Save app. `get_or_create_app` stores the description only when the
        // app is actually created; for an existing app it is left untouched
        // (admins can edit it later from the panel).
        let app = if data.is_new_app {
            db.custom_apps()
                .get_or_create_app(&data.slug, &data.name, data.description.as_deref(), user_id)
                .await?
        } else if let Some(app_id) = data.app_id {
            db.custom_apps().get_app_by_id(app_id).await?.unwrap()
        } else {
            db.custom_apps()
                .get_or_create_app(&data.slug, &data.name, data.description.as_deref(), user_id)
                .await?
        };

        // Check if author differs from the previous version of ANY status
        // (pending claims by other users must be visible to admins too).
        let prev_ver = db
            .custom_apps()
            .get_latest_version_any_status(app.id)
            .await?;
        let author_changed = match &prev_ver {
            Some(p) => p.submitted_by != user_id,
            None => false,
        };

        // Create version
        let ver_id = db
            .custom_apps()
            .create_version(
                app.id,
                &data.version,
                data.title.as_deref(),
                data.changelog.as_deref(),
                data.diff_url.as_deref(),
                data.cover_image_file_id.as_deref(),
                user_id,
            )
            .await?;

        // Add APK files
        for apk in &data.apk_files {
            let _ = db
                .custom_apps()
                .add_apk_file(
                    ver_id,
                    &apk.variant,
                    &apk.file_id,
                    &apk.file_unique_id,
                    apk.file_name.as_deref(),
                    apk.file_size,
                )
                .await;
        }

        // Save tags
        let _ = db
            .tags()
            .set_tags_for_item(ItemType::CustomApp, app.id, &data.tags)
            .await;

        dialogue.exit().await?;

        bot.send_message(
            chat_id,
            format!(
                "🎉 Заявка на публикацию приложения <b>{}</b> (v{}) успешно отправлена на модерацию администраторам!",
                encode_text(&data.name),
                encode_text(&data.version)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;

        // Notify all admins
        let kb = InlineKeyboardMarkup::new(vec![
            vec![
                InlineKeyboardButton::callback("✅ Одобрить", format!("apk_approve:{}", ver_id)),
                InlineKeyboardButton::callback("✏️ Редактировать", format!("apk_edit:{}", ver_id)),
            ],
            vec![InlineKeyboardButton::callback(
                "❌ Отклонить",
                format!("apk_reject:{}", ver_id),
            )],
        ]);

        let warning_note = if author_changed {
            "\n⚠️ <b>ВНИМАНИЕ: автор отличается от предыдущей версии!</b>\n"
        } else {
            ""
        };

        let user_info = match &username {
            Some(u) => format!("@{} (<code>{}</code>)", u, user_id),
            None => format!("<code>{}</code>", user_id),
        };

        let desc_note = match data.description.as_deref().map(str::trim) {
            Some(d) if !d.is_empty() => format!("\n📝 Описание: <i>{}</i>", encode_text(d)),
            _ => String::new(),
        };

        let admin_notice = format!(
            "📱 <b>Новая заявка на публикацию APK #{}</b>{}\n\
            📦 Приложение: <b>{}</b> (<code>{}</code>){}\n\
            🔖 Версия: <code>{}</code>\n\
            📌 Заголовок: <code>{}</code>\n\
            📝 Changelog: <code>{}</code>\n\
            🔗 Diff: <code>{}</code>\n\
            👤 Автор: {}\n\
            📁 Файлов APK: <b>{}</b>",
            ver_id,
            warning_note,
            encode_text(&data.name),
            encode_text(&data.slug),
            desc_note,
            encode_text(&data.version),
            encode_text(data.title.as_deref().unwrap_or("нет")),
            encode_text(data.changelog.as_deref().unwrap_or("нет")),
            encode_text(data.diff_url.as_deref().unwrap_or("нет")),
            user_info,
            data.apk_files.len()
        );

        super::notify_admins(
            bot,
            db,
            admin_notice,
            Some(kb),
            data.cover_image_file_id.as_deref(),
        )
        .await?;
    }
    Ok(())
}

pub async fn handle_submit_confirm_cancel(
    bot: &Bot,
    q: &CallbackQuery,
    dialogue: &BotDialogue,
) -> Result<()> {
    dialogue.exit().await?;
    if let Some(msg) = &q.message {
        let _ = bot
            .edit_message_text(msg.chat().id, msg.id(), crate::strings::CANCEL_MESSAGE)
            .await;
    }
    Ok(())
}

/// Anti-merge flow: the user chose to re-enter a different app name
/// after the "app with a similar name already exists" warning.
pub async fn handle_slug_rename(
    bot: &Bot,
    q: &CallbackQuery,
    dialogue: &BotDialogue,
) -> Result<()> {
    dialogue
        .update(DialogueState::SubmitApk(SubmitApkState::WaitingName))
        .await?;

    if let Some(msg) = &q.message {
        let _ = bot
            .edit_message_text(
                msg.chat().id,
                msg.id(),
                "✏️ Введите другое название приложения:",
            )
            .await;
    }
    Ok(())
}

/// Handles the "continue / restart / cancel" prompt shown when the user sends
/// `/submitapk` while an unfinished submission dialogue is already active.
pub async fn handle_submit_resume(
    bot: &Bot,
    q: &CallbackQuery,
    action: &str,
    dialogue: &BotDialogue,
) -> Result<()> {
    let Some(msg) = &q.message else {
        return Ok(());
    };

    match action {
        "continue" => {
            let _ = bot
                .answer_callback_query(q.id.clone())
                .text("▶️ Продолжайте с текущего шага — прогресс сохранен.")
                .await;
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    "▶️ Незавершенная заявка активна. Продолжайте с текущего шага (см. последнее сообщение мастера).\n\nЕсли шаг потерялся — отправьте <code>/submitapk</code> и выберите «Начать заново».",
                )
                .parse_mode(ParseMode::Html)
                .await;
        }
        "restart" => {
            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::ChoosingMode))
                .await?;
            let kb = InlineKeyboardMarkup::new(vec![
                vec![
                    InlineKeyboardButton::callback("🆕 Новое приложение", "submit_mode:new"),
                    InlineKeyboardButton::callback("🔄 Обновление существующего", "submit_mode:update"),
                ],
                vec![InlineKeyboardButton::callback(
                    "❌ Отмена",
                    "submit_confirm:cancel",
                )],
            ]);
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    "📱 <b>Мастер публикации приложения / APK</b>\n\nПрежние черновики сброшены. Выберите тип публикации:",
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(kb)
                .await;
        }
        "cancel" => {
            dialogue.exit().await?;
            let _ = bot
                .edit_message_text(msg.chat().id, msg.id(), crate::strings::CANCEL_MESSAGE)
                .await;
        }
        _ => {}
    }

    Ok(())
}
