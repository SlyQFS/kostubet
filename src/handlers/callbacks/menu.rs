//! Callback query handlers for menu navigation, start onboarding shortcuts, and APK downloads.

use crate::db::Database;
use crate::dialogue::state::{DialogueState, SuggestState};
use crate::dialogue::BotDialogue;
use crate::handlers::{commands_admin, commands_public};
use crate::strings::ACCESS_DENIED;
use anyhow::Result;
use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InputFile, ParseMode};

pub async fn handle_menu_action(
    bot: &Bot,
    q: &CallbackQuery,
    action: &str,
    db: &Database,
    user_id: i64,
) -> Result<()> {
    let chat_id = q
        .message
        .as_ref()
        .map(|m| m.chat().id)
        .unwrap_or(ChatId(user_id));

    if !db.admins().is_admin(user_id).await? {
        bot.send_message(chat_id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    match action {
        "list" => commands_admin::send_tools_list(bot, chat_id, db).await?,
        "tags" => commands_admin::send_tags_list(bot, chat_id, db).await?,
        "pending" => commands_admin::send_pending(bot, chat_id, db).await?,
        "admins" => commands_admin::send_admins_list(bot, chat_id, db).await?,
        _ => {}
    }

    Ok(())
}

pub async fn handle_start_action(
    bot: &Bot,
    q: &CallbackQuery,
    action: &str,
    dialogue: &BotDialogue,
    db: &Database,
    user_id: i64,
) -> Result<()> {
    let chat_id = q
        .message
        .as_ref()
        .map(|m| m.chat().id)
        .unwrap_or(ChatId(user_id));

    match action {
        "suggest" => {
            // Enter the button-driven suggestion flow (link -> confirm -> tags).
            dialogue
                .update(DialogueState::Suggest(SuggestState::WaitingLink))
                .await?;
            bot.send_message(
                chat_id,
                "💡 <b>Предложить репозиторий</b>\n\n\
                Отправьте ссылку на репозиторий GitHub:\n\
                <code>https://github.com/owner/repo</code> — или просто <code>owner/repo</code>\n\n\
                Отмена: <code>/cancel</code>",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        "submitapk" => {
            commands_public::start_submitapk(bot, chat_id, user_id, dialogue, db).await?;
        }
        "apps" => {
            commands_public::send_apps_list(bot, chat_id, db).await?;
        }
        "help" => {
            commands_public::send_help(bot, chat_id, user_id, db).await?;
        }
        _ => {}
    }

    Ok(())
}

pub async fn handle_apk_get(
    bot: &Bot,
    q: &CallbackQuery,
    file_row_id: i64,
    db: &Database,
) -> Result<()> {
    // This handler manages its own callback acknowledgement so that failures
    // can surface as error alerts instead of silently swallowed spinners.
    let Some(file_rec) = db.custom_apps().get_apk_file_by_id(file_row_id).await? else {
        let _ = bot
            .answer_callback_query(q.id.clone())
            .text("⚠️ Файл не найден в базе данных. Возможно, заявка была отклонена.")
            .show_alert(true)
            .await;
        return Ok(());
    };

    let Some(msg) = &q.message else {
        let _ = bot
            .answer_callback_query(q.id.clone())
            .text("⚠️ Не удалось отправить файл.")
            .show_alert(true)
            .await;
        return Ok(());
    };

    // Drop the loading spinner before the (possibly slow) document upload.
    let _ = bot.answer_callback_query(q.id.clone()).await;

    let doc = InputFile::file_id(file_rec.file_id);
    let mut send_req = bot.send_document(msg.chat().id, doc);
    if let Some(name) = file_rec.file_name {
        send_req = send_req
            .caption(format!(
                "📦 <b>{}</b> ({})",
                encode_text(&name),
                file_rec.variant_label
            ))
            .parse_mode(ParseMode::Html);
    }

    if let Err(e) = send_req.await {
        let _ = bot
            .send_message(
                msg.chat().id,
                format!("⚠️ Не удалось отправить файл: {}", e),
            )
            .await;
    }

    Ok(())
}
