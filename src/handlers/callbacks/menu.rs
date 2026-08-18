//! Callback query handlers for menu navigation and start onboarding shortcuts.

use crate::db::Database;
use crate::dialogue::state::{DialogueState, SuggestState};
use crate::dialogue::BotDialogue;
use crate::handlers::{commands_admin, commands_public};
use crate::strings::ACCESS_DENIED;
use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::{ChatId, ParseMode};

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
                "💡 Отправьте ссылку на репозиторий (например: <code>https://github.com/owner/example</code> или <code>owner/example</code>):",
            )
            .reply_markup(crate::dialogue::suggest::cancel_keyboard())
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


