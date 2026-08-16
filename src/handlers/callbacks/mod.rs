//! Callback query routing and shared moderation utilities.
//!
//! Submodules:
//! - `suggestions`: Suggestion approvals and rejections.
//! - `apk_moderation`: Custom APK approvals, rejections, and metadata editing.
//! - `submit_flow`: User APK wizard state transitions.
//! - `menu`: Inline navigation shortcuts and onboarding quick actions.
//! - `panel`: Admin panel (`adm:*`) and public paginated catalog (`pub:apps:*`).

pub mod apk_moderation;
pub mod menu;
pub mod panel;
pub mod submit_flow;
pub mod suggest_flow;
pub mod suggestions;

use crate::db::Database;
use crate::dialogue::BotDialogue;
use crate::strings::ACCESS_DENIED;
use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardMarkup, InputFile, ParseMode};
use tracing::{info, warn};

/// Verifies that the querying user has administrator privileges, replying with a localized alert if not.
pub async fn require_admin(
    bot: &Bot,
    q: &CallbackQuery,
    db: &Database,
    user_id: i64,
) -> Result<bool> {
    if db.admins().is_admin(user_id).await? {
        return Ok(true);
    }
    let _ = bot
        .answer_callback_query(q.id.clone())
        .text(ACCESS_DENIED)
        .await;
    Ok(false)
}

/// Broadcasts an administrative notification with optional action keyboard to
/// all registered bot admins. When `photo_file_id` is set, each admin first
/// receives the image as a preview (e.g. an APK card cover); a failed photo
/// send is logged and does not block the text notification.
pub async fn notify_admins(
    bot: &Bot,
    db: &Database,
    text: String,
    keyboard: Option<InlineKeyboardMarkup>,
    photo_file_id: Option<&str>,
) -> Result<()> {
    let admins = db.admins().list_admins().await.unwrap_or_default();
    for admin in admins {
        if let Some(file_id) = photo_file_id {
            let photo = bot
                .send_photo(ChatId(admin.telegram_id), InputFile::file_id(file_id.to_string()))
                .caption("🖼 Предпросмотр обложки из заявки");
            if let Err(e) = photo.await {
                warn!(
                    "Failed to send cover preview to admin {}: {:?}",
                    admin.telegram_id, e
                );
            }
        }

        let mut req = bot
            .send_message(ChatId(admin.telegram_id), text.clone())
            .parse_mode(ParseMode::Html);

        if let Some(ref kb) = keyboard {
            req = req.reply_markup(kb.clone());
        }

        if let Err(e) = req.await {
            warn!("Failed to notify admin {}: {:?}", admin.telegram_id, e);
        }
    }
    Ok(())
}

#[tracing::instrument(skip(bot, dialogue, db))]
pub async fn handle_callback(
    bot: Bot,
    q: CallbackQuery,
    dialogue: BotDialogue,
    db: Database,
    target_chat_id: i64,
    target_thread_id: Option<i64>,
) -> Result<()> {
    let Some(data) = q.data.clone() else {
        return Ok(());
    };

    let user = q.from.clone();
    let user_id = user.id.0 as i64;
    let username = user.username.clone();
    let admin_name = match &username {
        Some(u) => format!("@{}", u),
        None => format!("<code>{}</code>", user_id),
    };

    // Acknowledge callback immediately to remove loading spinner in Telegram client.
    let _ = bot.answer_callback_query(q.id.clone()).await;

    // 1. Repo Suggestion Moderation
    if let Some(id_str) = data.strip_prefix("suggest_approve:") {
        let Ok(sugg_id) = id_str.parse::<i64>() else {
            return Ok(());
        };
        if !require_admin(&bot, &q, &db, user_id).await? {
            return Ok(());
        }
        return suggestions::handle_suggestion_approve(
            &bot,
            &q,
            sugg_id,
            &db,
            user_id,
            &admin_name,
        )
        .await;
    }

    if let Some(id_str) = data.strip_prefix("suggest_reject:") {
        let Ok(sugg_id) = id_str.parse::<i64>() else {
            return Ok(());
        };
        if !require_admin(&bot, &q, &db, user_id).await? {
            return Ok(());
        }
        return suggestions::handle_suggestion_reject(&bot, &q, sugg_id, &db, user_id, &admin_name)
            .await;
    }

    // 2. APK Version Moderation & Editing
    if let Some(id_str) = data.strip_prefix("apk_approve:") {
        let Ok(ver_id) = id_str.parse::<i64>() else {
            return Ok(());
        };
        if !require_admin(&bot, &q, &db, user_id).await? {
            return Ok(());
        }
        return apk_moderation::handle_apk_approve(
            &bot,
            &q,
            ver_id,
            &db,
            user_id,
            &admin_name,
            target_chat_id,
            target_thread_id,
        )
        .await;
    }

    if let Some(id_str) = data.strip_prefix("apk_reject:") {
        let Ok(ver_id) = id_str.parse::<i64>() else {
            return Ok(());
        };
        if !require_admin(&bot, &q, &db, user_id).await? {
            return Ok(());
        }
        return apk_moderation::handle_apk_reject(&bot, &q, ver_id, &db, user_id, &admin_name)
            .await;
    }

    if let Some(id_str) = data.strip_prefix("apk_edit:") {
        let Ok(ver_id) = id_str.parse::<i64>() else {
            return Ok(());
        };
        if !require_admin(&bot, &q, &db, user_id).await? {
            return Ok(());
        }
        return apk_moderation::handle_apk_edit_start(&bot, &q, ver_id, &dialogue, &db).await;
    }

    if let Some(id_str) = data.strip_prefix("edit_publish:") {
        let Ok(ver_id) = id_str.parse::<i64>() else {
            return Ok(());
        };
        if !require_admin(&bot, &q, &db, user_id).await? {
            return Ok(());
        }
        return apk_moderation::handle_apk_edit_publish(
            &bot,
            &q,
            ver_id,
            &dialogue,
            &db,
            user_id,
            target_chat_id,
            target_thread_id,
        )
        .await;
    }

    if data == "edit_cancel" {
        return apk_moderation::handle_apk_edit_cancel(&bot, &q, &dialogue).await;
    }

    // 3. User Submit APK Flow
    if data == "submit_mode:new" {
        return submit_flow::handle_submit_mode_new(&bot, &q, &dialogue).await;
    }

    if data == "submit_mode:update" {
        return submit_flow::handle_submit_mode_update(&bot, &q, &dialogue, &db).await;
    }

    if let Some(slug) = data.strip_prefix("submit_app:") {
        return submit_flow::handle_submit_app_select(&bot, &q, slug, &dialogue, &db).await;
    }

    if let Some(variant) = data.strip_prefix("variant_select:") {
        return submit_flow::handle_variant_select(&bot, &q, variant, &dialogue).await;
    }

    if data == "submit_confirm:send" {
        return submit_flow::handle_submit_confirm_send(
            &bot, &q, &dialogue, &db, user_id, username,
        )
        .await;
    }

    if data == "submit_confirm:cancel" {
        return submit_flow::handle_submit_confirm_cancel(&bot, &q, &dialogue).await;
    }

    // Resume prompt when /submitapk is invoked mid-dialogue
    if let Some(action) = data.strip_prefix("submitresume:") {
        return submit_flow::handle_submit_resume(&bot, &q, action, &dialogue).await;
    }

    // Anti-merge flow: re-enter the app name
    if data == "submitslug:rename" {
        return submit_flow::handle_slug_rename(&bot, &q, &dialogue).await;
    }

    // Button-driven repository suggestion flow
    if let Some(action) = data.strip_prefix("sugg_") {
        return suggest_flow::handle_suggest_flow(
            &bot, &q, action, &dialogue, &db, user_id, username,
        )
        .await;
    }

    // 4. Admin Panel (adm:*) and public catalog (pub:apps:* / appcard:*)
    if let Some(rest) = data.strip_prefix("adm:") {
        return panel::handle_panel_callback(&bot, &q, rest, &dialogue, &db, user_id, target_chat_id)
            .await;
    }

    if let Some(p) = data.strip_prefix("pub:apps:") {
        let page = p.parse::<usize>().unwrap_or(0);
        return panel::handle_apps_page_callback(&bot, &q, page, &db).await;
    }

    if let Some(slug) = data.strip_prefix("appcard:") {
        return panel::handle_appcard_callback(&bot, &q, slug, &db).await;
    }

    // 5. Menu / Start Actions
    if let Some(action) = data.strip_prefix("menu:") {
        return menu::handle_menu_action(&bot, &q, action, &db, user_id).await;
    }

    if let Some(action) = data.strip_prefix("start:") {
        return menu::handle_start_action(&bot, &q, action, &dialogue, &db, user_id).await;
    }

    info!("Unhandled callback query data: {}", data);
    Ok(())
}
