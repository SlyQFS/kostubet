//! Telegram command and callback dispatchers.
//!
//! Submodules:
//! - `commands_public`: Public command handlers (`/start`, `/help`, `/suggest`, etc.)
//! - `commands_admin`: Administrator and owner handlers (`/track`, `/list`, `/debug`, etc.)
//! - `callbacks`: Callback query handlers organized by flow.

pub mod callbacks;
pub mod commands_admin;
pub mod commands_public;

use crate::db::Database;
use crate::dialogue::BotDialogue;
use crate::strings::GROUP_NOTICE_HTML;
use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::{MessageId, ParseMode, ThreadId};
use teloxide::utils::command::BotCommands;
use tracing::error;

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Команды бота Kostubet:")]
pub enum Command {
    #[command(description = "Показать справку по командам.")]
    Help,
    #[command(description = "Начать работу с ботом.")]
    Start,
    #[command(description = "Открыть панель администратора.")]
    Admin,
    #[command(description = "Отменить текущий диалог (мастер подачи заявки).")]
    Cancel,
    #[command(description = "Техническая диагностика бота (только владелец).")]
    Debug,
    #[command(description = "Отправить тестовую карточку релиза в группу.")]
    Test,
    #[command(description = "Добавить администратора. Использование: /addadmin <id>")]
    Addadmin(String),
    #[command(description = "Удалить администратора. Использование: /removeadmin <id>")]
    Removeadmin(String),
    #[command(description = "Список администраторов бота.")]
    Admins,
    #[command(description = "Добавить репозиторий. Использование: /track owner/repo [#tag ...]")]
    Track(String),
    #[command(description = "Удалить репозиторий. Использование: /untrack owner/repo")]
    Untrack(String),
    #[command(description = "Добавить тег инструменту. Использование: /addtag owner/repo #tag")]
    Addtag(String),
    #[command(
        description = "Удалить тег у инструмента. Использование: /removetag owner/repo #tag"
    )]
    Removetag(String),
    #[command(description = "Список отслеживаемых инструментов и их тегов.")]
    List,
    #[command(description = "Канонический список всех тегов.")]
    Tags,
    #[command(description = "Все заявки на модерацию (репозитории и APK).")]
    Pending,
    #[command(
        description = "Предложить репозиторий (ссылка или owner/repo). Без аргументов — кнопочный мастер"
    )]
    Suggest(String),
    #[command(description = "Статус ваших предложенных заявок.")]
    Mysuggestions,
    #[command(description = "Предложить новое приложение или апдейт (APK).")]
    Submitapk,
    #[command(description = "Список опубликованных кастомных приложений.")]
    Apps,
}

#[tracing::instrument(skip(bot, dialogue, db))]
pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    dialogue: BotDialogue,
    db: Database,
    target_chat_id: i64,
    target_thread_id: Option<i64>,
) -> ResponseResult<()> {
    // Restrict bot commands exclusively to Direct Messages (DM / ЛС)
    if !msg.chat.is_private() {
        let send_group_notice = |text: String| {
            let bot = bot.clone();
            let chat_id = msg.chat.id;
            let thread_id = msg.thread_id.map(|t| ThreadId(MessageId(t.0 .0)));
            async move {
                let mut req = bot.send_message(chat_id, text).parse_mode(ParseMode::Html);
                if let Some(tid) = thread_id {
                    req = req.message_thread_id(tid);
                }
                let _ = req.await;
            }
        };
        send_group_notice(GROUP_NOTICE_HTML.to_string()).await;
        return Ok(());
    }

    let res: Result<()> = match cmd {
        Command::Start => commands_public::handle_start(&bot, &msg).await,
        Command::Help => commands_public::handle_help(&bot, &msg, &db).await,
        Command::Admin => commands_public::handle_admin_panel(&bot, &msg, &db).await,
        Command::Cancel => {
            let _ = dialogue.exit().await;
            bot.send_message(msg.chat.id, crate::strings::CANCEL_MESSAGE).await?;
            Ok(())
        }
        Command::Debug => {
            commands_admin::handle_debug(&bot, &msg, &db, target_chat_id, target_thread_id).await
        }
        Command::Test => {
            commands_admin::handle_test(&bot, &msg, &db, target_chat_id, target_thread_id).await
        }
        Command::Addadmin(args) => commands_admin::handle_addadmin(&bot, &msg, &args, &db).await,
        Command::Removeadmin(args) => {
            commands_admin::handle_removeadmin(&bot, &msg, &args, &db).await
        }
        Command::Admins => commands_admin::handle_admins(&bot, &msg, &db).await,
        Command::Track(args) => commands_admin::handle_track(&bot, &msg, &args, &db).await,
        Command::Untrack(args) => commands_admin::handle_untrack(&bot, &msg, &args, &db).await,
        Command::Addtag(args) => commands_admin::handle_addtag(&bot, &msg, &args, &db).await,
        Command::Removetag(args) => commands_admin::handle_removetag(&bot, &msg, &args, &db).await,
        Command::List => commands_admin::handle_list(&bot, &msg, &db).await,
        Command::Tags => commands_admin::handle_tags(&bot, &msg, &db).await,
        Command::Pending => commands_admin::handle_pending(&bot, &msg, &db).await,
        Command::Suggest(args) => {
            commands_public::handle_suggest(&bot, &msg, &args, &dialogue, &db).await
        }
        Command::Mysuggestions => commands_public::handle_mysuggestions(&bot, &msg, &db).await,
        Command::Submitapk => commands_public::handle_submitapk(&bot, &msg, &dialogue, &db).await,
        Command::Apps => commands_public::handle_apps(&bot, &msg, &db).await,
    };

    if let Err(e) = res {
        error!("Error executing command: {:?}", e);
    }

    Ok(())
}
