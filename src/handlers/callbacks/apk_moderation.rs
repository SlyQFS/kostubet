//! Callback query handlers for APK application review, publishing, and editing.

use crate::db::tags::ItemType;
use crate::db::Database;
use crate::dialogue::state::{DialogueState, EditApkData, EditApkState};
use crate::dialogue::BotDialogue;
use crate::services::render::{build_apk_post_data, send_apk_documents, send_post};
use anyhow::Result;
use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{ChatId, ParseMode};
use tracing::{error, info, warn};

#[allow(clippy::too_many_arguments)]
pub async fn handle_apk_approve(
    bot: &Bot,
    q: &CallbackQuery,
    ver_id: i64,
    db: &Database,
    user_id: i64,
    admin_name: &str,
    target_chat_id: i64,
    target_thread_id: Option<i64>,
) -> Result<()> {
    let Some(ver) = db.custom_apps().get_version(ver_id).await? else {
        return Ok(());
    };
    let Some(app) = db.custom_apps().get_app_by_id(ver.app_id).await? else {
        return Ok(());
    };

    let updated = db
        .custom_apps()
        .set_version_status_if_pending(ver_id, "approved", user_id, None)
        .await?;

    if !updated {
        if let Some(msg) = &q.message {
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    format!(
                        "⚠️ Заявка на APK #{} уже была обработана другим администратором.",
                        ver_id
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await;
        }
        return Ok(());
    }

    // Build the publication post
    let apk_files = db.custom_apps().get_apk_files(ver_id).await?;

    let tags = db
        .tags()
        .get_tags_for_item(ItemType::CustomApp, app.id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.name)
        .collect();

    let post = build_apk_post_data(
        &app.name,
        &ver.version,
        app.description.clone(),
        ver.changelog.clone(),
        ver.diff_url.clone(),
        ver.cover_image_file_id.clone(),
        tags,
        ver.submitted_by_username.clone(),
    );

    // Publish; on failure roll the version back to `pending` so it returns
    // to the moderation queue instead of hanging as approved-but-unpublished.
    if target_chat_id == 0 {
        let _ = db.custom_apps().reset_version_to_pending(ver_id).await;
        error!("Cannot publish APK version {}: TELEGRAM_CHAT_ID is not configured", ver_id);
        if let Some(msg) = &q.message {
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    format!(
                        "❌ Не удалось опубликовать заявку #{}: <b>TELEGRAM_CHAT_ID</b> не настроен. Заявка возвращена в очередь модерации.",
                        ver_id
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await;
        }
        return Ok(());
    }

    let published_msg = match send_post(bot, target_chat_id, target_thread_id, &post).await {
        Ok(m) => m,
        Err(e) => {
            let _ = db.custom_apps().reset_version_to_pending(ver_id).await;
            error!("Failed to publish custom app APK post: {:?}", e);
            if let Some(msg) = &q.message {
                let _ = bot
                    .edit_message_text(
                        msg.chat().id,
                        msg.id(),
                        format!(
                            "❌ Ошибка публикации заявки #{}: {}. Заявка возвращена в очередь модерации — попробуйте еще раз.",
                            ver_id, e
                        ),
                    )
                    .await;
            }
            return Ok(());
        }
    };

    info!(
        "Published custom app APK {} v{} to Telegram chat {}",
        app.name, ver.version, target_chat_id
    );

    // Deliver the APK files as documents right after the card. A delivery
    // failure must not roll the version back (the card is already published);
    // surface it to the admin instead.
    let delivery_errors =
        send_apk_documents(bot, target_chat_id, target_thread_id, &apk_files, &app.name, &ver.version)
            .await;
    if !delivery_errors.is_empty() {
        warn!(
            "Failed to deliver some APK files for version {}: {:?}",
            ver_id, delivery_errors
        );
    }

    let _ = db
        .audit()
        .log_action(
            user_id,
            "одобрил и опубликовал APK",
            &format!("{} v{}", app.name, ver.version),
        )
        .await;

    db.custom_apps()
        .set_app_current_version(app.id, ver_id)
        .await?;

    // Unconditionally store published_message_id in DB
    if let Err(e) = db
        .custom_apps()
        .set_published_message_id(ver_id, published_msg.id.0 as i64)
        .await
    {
        error!(
            "Failed to save published_message_id for APK version {}: {:?}",
            ver_id, e
        );
    }

    if let Some(msg) = &q.message {
        let delivery_note = if delivery_errors.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n⚠️ Не удалось доставить некоторые APK-файлы: {}. Файлы можно отправить повторно из каталога /apps.",
                delivery_errors.join("; ")
            )
        };
        let _ = bot
            .edit_message_text(
                msg.chat().id,
                msg.id(),
                format!(
                    "✅ Заявка на APK #{} (<b>{} v{}</b>) одобрена администратором {} и опубликована!{}",
                    ver_id,
                    encode_text(&app.name),
                    encode_text(&ver.version),
                    admin_name,
                    delivery_note
                ),
            )
            .parse_mode(ParseMode::Html)
            .await;
    }

    // Notify author
    if let Err(e) = bot
        .send_message(
            ChatId(ver.submitted_by),
            format!(
                "🎉 Ваша заявка на публикацию приложения <b>{}</b> (v{}) одобрена и опубликована!",
                encode_text(&app.name),
                encode_text(&ver.version)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await
    {
        warn!("Failed to notify APK author {}: {:?}", ver.submitted_by, e);
    }

    Ok(())
}

pub async fn handle_apk_reject(
    bot: &Bot,
    q: &CallbackQuery,
    ver_id: i64,
    db: &Database,
    user_id: i64,
    admin_name: &str,
) -> Result<()> {
    let Some(ver) = db.custom_apps().get_version(ver_id).await? else {
        return Ok(());
    };
    let Some(app) = db.custom_apps().get_app_by_id(ver.app_id).await? else {
        return Ok(());
    };

    let updated = db
        .custom_apps()
        .set_version_status_if_pending(ver_id, "rejected", user_id, None)
        .await?;

    if !updated {
        if let Some(msg) = &q.message {
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    format!(
                        "⚠️ Заявка на APK #{} уже была обработана другим администратором.",
                        ver_id
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await;
        }
        return Ok(());
    }

    let _ = db
        .audit()
        .log_action(
            user_id,
            "отклонил APK",
            &format!("{} v{}", app.name, ver.version),
        )
        .await;

    if let Some(msg) = &q.message {
        let _ = bot
            .edit_message_text(
                msg.chat().id,
                msg.id(),
                format!(
                    "❌ Заявка на APK #{} (<b>{} v{}</b>) отклонена администратором {}.",
                    ver_id,
                    encode_text(&app.name),
                    encode_text(&ver.version),
                    admin_name
                ),
            )
            .parse_mode(ParseMode::Html)
            .await;
    }

    // Notify author
    if let Err(e) = bot
        .send_message(
            ChatId(ver.submitted_by),
            format!(
                "❌ Ваша заявка на публикацию приложения <b>{}</b> (v{}) была отклонена администратором.",
                encode_text(&app.name),
                encode_text(&ver.version)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await
    {
        warn!("Failed to notify APK author {}: {:?}", ver.submitted_by, e);
    }

    Ok(())
}

pub async fn handle_apk_edit_start(
    bot: &Bot,
    q: &CallbackQuery,
    ver_id: i64,
    dialogue: &BotDialogue,
    db: &Database,
) -> Result<()> {
    let Some(ver) = db.custom_apps().get_version(ver_id).await? else {
        return Ok(());
    };
    let Some(app) = db.custom_apps().get_app_by_id(ver.app_id).await? else {
        return Ok(());
    };
    let tags = db
        .tags()
        .get_tags_for_item(ItemType::CustomApp, app.id)
        .await
        .unwrap_or_default();

    let edit_data = Box::new(EditApkData {
        version_id: ver.id,
        app_name: app.name,
        version: ver.version,
        description: app.description,
        title: ver.title,
        changelog: ver.changelog,
        diff_url: ver.diff_url,
        cover_image_file_id: ver.cover_image_file_id,
        tags: tags.into_iter().map(|t| t.name).collect(),
        submitted_by_username: ver.submitted_by_username,
    });

    let cur_title = edit_data
        .title
        .clone()
        .unwrap_or_else(|| "не указан".to_string());

    dialogue
        .update(DialogueState::EditApk(EditApkState::EditingTitle {
            data: edit_data,
        }))
        .await?;

    if let Some(msg) = &q.message {
        bot.send_message(
            msg.chat().id,
            format!(
                "✏️ <b>Редактирование заявки #{}</b>\n\
                Текущий заголовок: <code>{}</code>\n\n\
                Введите новый заголовок, <code>/skip</code> — оставить как есть, <code>/clear</code> — убрать:",
                ver_id,
                encode_text(&cur_title)
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_apk_edit_publish(
    bot: &Bot,
    q: &CallbackQuery,
    ver_id: i64,
    dialogue: &BotDialogue,
    db: &Database,
    user_id: i64,
    target_chat_id: i64,
    target_thread_id: Option<i64>,
) -> Result<()> {
    let cur_state = dialogue.get().await?;
    if let Some(DialogueState::EditApk(EditApkState::ConfirmEdit { data })) = cur_state {
        let Some(ver) = db.custom_apps().get_version(ver_id).await? else {
            return Ok(());
        };
        let Some(app) = db.custom_apps().get_app_by_id(ver.app_id).await? else {
            return Ok(());
        };

        // Atomic claim guard FIRST: an already-processed version must never
        // have its fields/tags overwritten by a late edit.
        let updated = db
            .custom_apps()
            .set_version_status_if_pending(ver_id, "approved", user_id, None)
            .await?;
        if !updated {
            dialogue.exit().await?;
            if let Some(msg) = &q.message {
                let _ = bot
                    .edit_message_text(
                        msg.chat().id,
                        msg.id(),
                        format!(
                            "⚠️ Заявка на APK #{} уже была обработана другим администратором.",
                            ver_id
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await;
            }
            return Ok(());
        }

        // Update DB version and tags (only after the claim succeeded)
        db.custom_apps()
            .update_version_fields(
                ver_id,
                data.title.as_deref(),
                data.changelog.as_deref(),
                data.diff_url.as_deref(),
                data.cover_image_file_id.as_deref(),
            )
            .await?;

        db.custom_apps()
            .set_app_description(app.id, data.description.as_deref())
            .await?;

        db.tags()
            .set_tags_for_item(ItemType::CustomApp, app.id, &data.tags)
            .await?;

        // Publish post
        let apk_files = db.custom_apps().get_apk_files(ver_id).await?;

        let post = build_apk_post_data(
            &app.name,
            &ver.version,
            data.description.clone(),
            data.changelog.clone(),
            data.diff_url.clone(),
            data.cover_image_file_id.clone(),
            data.tags.clone(),
            data.submitted_by_username.clone(),
        );

        if target_chat_id == 0 {
            let _ = db.custom_apps().reset_version_to_pending(ver_id).await;
            dialogue.exit().await?;
            if let Some(msg) = &q.message {
                let _ = bot
                    .edit_message_text(
                        msg.chat().id,
                        msg.id(),
                        format!(
                            "❌ Не удалось опубликовать заявку #{}: <b>TELEGRAM_CHAT_ID</b> не настроен. Заявка возвращена в очередь.",
                            ver_id
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await;
            }
            return Ok(());
        }

        let published_msg = match send_post(bot, target_chat_id, target_thread_id, &post).await {
            Ok(m) => m,
            Err(e) => {
                let _ = db.custom_apps().reset_version_to_pending(ver_id).await;
                dialogue.exit().await?;
                error!("Failed to publish edited APK post: {:?}", e);
                if let Some(msg) = &q.message {
                    let _ = bot
                        .edit_message_text(
                            msg.chat().id,
                            msg.id(),
                            format!(
                                "❌ Ошибка публикации заявки #{}: {}. Заявка возвращена в очередь модерации.",
                                ver_id, e
                            ),
                        )
                        .await;
                }
                return Ok(());
            }
        };

        db.custom_apps()
            .set_app_current_version(app.id, ver_id)
            .await?;
        let _ = db
            .custom_apps()
            .set_published_message_id(ver_id, published_msg.id.0 as i64)
            .await;

        // Deliver the APK files as documents right after the card.
        let delivery_errors = send_apk_documents(
            bot,
            target_chat_id,
            target_thread_id,
            &apk_files,
            &app.name,
            &ver.version,
        )
        .await;
        if !delivery_errors.is_empty() {
            warn!(
                "Failed to deliver some APK files for version {}: {:?}",
                ver_id, delivery_errors
            );
        }

        let _ = db
            .audit()
            .log_action(
                user_id,
                "опубликовал APK (после правки)",
                &format!("{} v{}", app.name, ver.version),
            )
            .await;

        dialogue.exit().await?;

        if let Some(msg) = &q.message {
            let delivery_note = if delivery_errors.is_empty() {
                String::new()
            } else {
                format!(
                    "\n\n⚠️ Не удалось доставить некоторые APK-файлы: {}. Файлы можно отправить повторно из каталога /apps.",
                    delivery_errors.join("; ")
                )
            };
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    format!(
                        "🎉 Отредактированная версия #{} (<b>{} v{}</b>) успешно опубликована!{}",
                        ver_id,
                        encode_text(&app.name),
                        encode_text(&ver.version),
                        delivery_note
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await;
        }

        // Notify author
        if let Err(e) = bot
            .send_message(
                ChatId(ver.submitted_by),
                format!(
                    "🎉 Ваша заявка на публикацию приложения <b>{}</b> (v{}) одобрена и опубликована!",
                    encode_text(&app.name),
                    encode_text(&ver.version)
                ),
            )
            .parse_mode(ParseMode::Html)
            .await
        {
            warn!("Failed to notify APK author {}: {:?}", ver.submitted_by, e);
        }
    }

    Ok(())
}

pub async fn handle_apk_edit_cancel(
    bot: &Bot,
    q: &CallbackQuery,
    dialogue: &BotDialogue,
) -> Result<()> {
    dialogue.exit().await?;
    if let Some(msg) = &q.message {
        let _ = bot
            .edit_message_text(
                msg.chat().id,
                msg.id(),
                "❌ Редактирование заявки отменено.",
            )
            .await;
    }
    Ok(())
}
