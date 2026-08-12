use crate::config::RepoConfig;
use crate::db::Database;
use crate::formatter::format_update_message;
use crate::github::{GithubUpdate, UpdateType};
use anyhow::Result;
use teloxide::errors::RequestError;
use teloxide::prelude::*;
use teloxide::types::{ChatId, LinkPreviewOptions, MessageId, ParseMode, ThreadId};
use teloxide::utils::command::BotCommands;
use tracing::{error, info, warn};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Kostubet Github Watcher Bot commands:")]
pub enum Command {
    #[command(description = "Display help and chat IDs.")]
    Help,
    #[command(description = "Start bot interaction.")]
    Start,
    #[command(description = "Open Admin Panel.")]
    Admin,
    #[command(description = "Send a test release card to your Telegram group/topic thread.")]
    Test,
    #[command(description = "Track a repository. Usage: /track owner/repo")]
    Track(String),
    #[command(description = "Untrack a repository. Usage: /untrack owner/repo")]
    Untrack(String),
    #[command(description = "List currently tracked repositories.")]
    List,
}

fn disabled_link_preview() -> LinkPreviewOptions {
    LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    }
}

pub async fn is_user_admin(msg: &Message, admin_user_ids: &[i64]) -> bool {
    let user = match msg.from.as_ref() {
        Some(u) => u,
        None => return false,
    };

    let user_id = user.id.0 as i64;

    // If admin_user_ids is empty, allow DM user
    if admin_user_ids.is_empty() {
        return true;
    }

    admin_user_ids.contains(&user_id)
}

pub async fn send_update(
    bot: &Bot,
    chat_id: i64,
    thread_id: Option<i64>,
    html_text: &str,
) -> Result<()> {
    let mut retries = 0;
    let max_retries = 3;

    loop {
        let mut request = bot
            .send_message(ChatId(chat_id), html_text)
            .parse_mode(ParseMode::Html)
            .link_preview_options(disabled_link_preview());

        if let Some(tid) = thread_id {
            request = request.message_thread_id(ThreadId(MessageId(tid as i32)));
        }

        match request.await {
            Ok(_) => return Ok(()),
            Err(RequestError::RetryAfter(seconds)) => {
                warn!("Telegram rate limit hit! Waiting for {:?} before retrying...", seconds);
                tokio::time::sleep(seconds.duration()).await;
                retries += 1;
                if retries > max_retries {
                    anyhow::bail!("Telegram send failed after {} flood wait retries", max_retries);
                }
            }
            Err(e) => {
                anyhow::bail!("Failed to send message to Telegram: {}", e);
            }
        }
    }
}

pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    db: Database,
    target_chat_id: i64,
    target_thread_id: Option<i64>,
    admin_user_ids: Vec<i64>,
) -> ResponseResult<()> {
    // Restrict commands exclusively to Direct Messages (DM / ЛС) to prevent group chat clutter
    if !msg.chat.is_private() {
        let send_group_notice = |text: String| {
            let bot = bot.clone();
            let chat_id = msg.chat.id;
            let thread_id = msg.thread_id.map(|t| ThreadId(MessageId(t.0.0 as i32)));
            async move {
                let mut req = bot
                    .send_message(chat_id, text)
                    .parse_mode(ParseMode::Html)
                    .link_preview_options(disabled_link_preview());
                if let Some(tid) = thread_id {
                    req = req.message_thread_id(tid);
                }
                let _ = req.await;
            }
        };
        send_group_notice("🔒 <i>Бот управляется локально в личных сообщениях (ЛС). Отправьте команду мне в личку!</i>".to_string()).await;
        return Ok(());
    }

    let send_reply = |text: String| {
        let bot = bot.clone();
        let chat_id = msg.chat.id;
        async move {
            let req = bot
                .send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .link_preview_options(disabled_link_preview());
            if let Err(e) = req.await {
                error!("Failed to send reply message to Telegram DM: {:?}", e);
            }
        }
    };

    match cmd {
        Command::Help | Command::Start => {
            let user_id_info = msg
                .from
                .as_ref()
                .map(|u| u.id.0.to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let help_text = format!(
                "<b>Kostubet GitHub Watcher Bot 🚀 (Локальное управление в ЛС)</b>\n\n\
                <b>Команды управления (только в ЛС):</b>\n\
                • <code>/admin</code> - Панель администратора\n\
                • <code>/test</code> - Отправить тестовую карточку релиза в ваш топик группы\n\
                • <code>/track &lt;owner/repo&gt;</code> - Добавить репозиторий для отслеживания\n\
                • <code>/untrack &lt;owner/repo&gt;</code> - Удалить репозиторий\n\
                • <code>/list</code> - Показать отслеживаемые репозитории\n\
                • <code>/help</code> - Показать эту справку\n\n\
                <b>Ваши данные для настройки:</b>\n\
                Ваш Telegram User ID: <code>{}</code>\n\
                Целевой Chat ID группы: <code>{}</code>\n\
                Целевой Topic Thread ID: <code>{}</code>",
                user_id_info,
                target_chat_id,
                target_thread_id.map(|t| t.to_string()).unwrap_or_else(|| "Не задан".to_string())
            );
            send_reply(help_text).await;
        }
        Command::Admin => {
            if !is_user_admin(&msg, &admin_user_ids).await {
                send_reply("⛔ <b>Доступ запрещен!</b> Вы не являетесь администратором.".to_string()).await;
                return Ok(());
            }

            let repo_count = match db.get_tracked_repos().await {
                Ok(repos) => repos.len(),
                Err(_) => 0,
            };

            let admin_panel_text = format!(
                "👑 <b>Локальная Панель Администратора</b>\n\n\
                📊 <b>Статус бота:</b> 🟢 Активен\n\
                📦 <b>Отслеживается репозиториев:</b> <code>{}</code>\n\
                🎯 <b>Целевой Chat ID группы:</b> <code>{}</code>\n\
                📌 <b>Целевой Topic Thread ID:</b> <code>{}</code>\n\n\
                <b>Быстрый тест и управление:</b>\n\
                • <code>/test</code> — Отправить Rich Text тестовую карточку релиза в группу\n\
                • <code>/track owner/repo</code> — Добавить репозиторий\n\
                • <code>/untrack owner/repo</code> — Удалить репозиторий\n\
                • <code>/list</code> — Список репозиториев",
                repo_count,
                target_chat_id,
                target_thread_id.map(|t| t.to_string()).unwrap_or_else(|| "Не задан".to_string())
            );
            send_reply(admin_panel_text).await;
        }
        Command::Test => {
            if !is_user_admin(&msg, &admin_user_ids).await {
                send_reply("⛔ <b>Доступ запрещен!</b>".to_string()).await;
                return Ok(());
            }

            if target_chat_id == 0 {
                send_reply("⚠️ Невозможно отправить карточку: <b>TELEGRAM_CHAT_ID</b> равен 0! Укажите TELEGRAM_CHAT_ID в .env.".to_string()).await;
                return Ok(());
            }

            let sample_update = GithubUpdate {
                update_type: UpdateType::Release,
                id: "test-v1.42.0".to_string(),
                tag_or_version: "v1.42.0".to_string(),
                title: "Tokio v1.42.0 - Asynchronous I/O Framework".to_string(),
                url: "https://github.com/tokio-rs/tokio/releases/tag/tokio-1.42.0".to_string(),
                body: Some(
                    "# Tokio v1.42.0 Highlights\n\n\
                    - **Async Scheduling**: Improved task scheduling algorithms\n\
                    - *Linux Drivers*: Enhanced `io_uring` zero-copy I/O drivers\n\
                    - `tokio::time`: High precision timer resolution\n\n\
                    ```rust\n\
                    #[tokio::main]\n\
                    async fn main() {\n\
                        println!(\"Hello Tokio!\");\n\
                    }\n\
                    ```\n\n\
                    For details see [Tokio Documentation](https://tokio.rs)".to_string(),
                ),
                etag: None,
                sha: None,
            };

            let test_card_html = format_update_message("tokio-rs", "tokio", &sample_update, true);

            match send_update(&bot, target_chat_id, target_thread_id, &test_card_html).await {
                Ok(_) => {
                    let dest = match target_thread_id {
                        Some(tid) => format!("Группа <code>{}</code> (Топик <code>{}</code>)", target_chat_id, tid),
                        None => format!("Группа <code>{}</code>", target_chat_id),
                    };
                    send_reply(format!("✅ <b>Тестовая Rich Text карточка отправлена в Telegram!</b>\nНазначение: {}", dest)).await;
                }
                Err(e) => {
                    send_reply(format!("❌ <b>Ошибка отправки тестовой карточки:</b> {}", e)).await;
                }
            }
        }
        Command::Track(repo_str) => {
            if !is_user_admin(&msg, &admin_user_ids).await {
                send_reply("⛔ <b>Доступ запрещен!</b>".to_string()).await;
                return Ok(());
            }

            let repo_str = repo_str.trim();
            if let Some(repo) = RepoConfig::parse(repo_str) {
                match db.add_repo(&repo.owner, &repo.name).await {
                    Ok(true) => {
                        info!("Started tracking repo: {}", repo.full_name());
                        send_reply(format!("✅ Репозиторий <b>{}</b> добавлен в отслеживание!", repo.full_name())).await;
                    }
                    Ok(false) => {
                        send_reply(format!("ℹ️ Репозиторий <b>{}</b> уже отслеживается.", repo.full_name())).await;
                    }
                    Err(e) => {
                        error!("Failed to add repo to DB: {:?}", e);
                        send_reply(format!("❌ Ошибка добавления <b>{}</b>: {}", repo_str, e)).await;
                    }
                }
            } else {
                send_reply(
                    "❌ Неверный формат! Указывайте репозиторий как <code>owner/repo</code>, например:\n<code>/track tokio-rs/tokio</code>".to_string()
                ).await;
            }
        }
        Command::Untrack(repo_str) => {
            if !is_user_admin(&msg, &admin_user_ids).await {
                send_reply("⛔ <b>Доступ запрещен!</b>".to_string()).await;
                return Ok(());
            }

            let repo_str = repo_str.trim();
            if let Some(repo) = RepoConfig::parse(repo_str) {
                match db.remove_repo(&repo.owner, &repo.name).await {
                    Ok(true) => {
                        info!("Stopped tracking repo: {}", repo.full_name());
                        send_reply(format!("🗑️ Репозиторий <b>{}</b> удален из отслеживания.", repo.full_name())).await;
                    }
                    Ok(false) => {
                        send_reply(format!("ℹ️ Репозиторий <b>{}</b> не был в списке.", repo.full_name())).await;
                    }
                    Err(e) => {
                        error!("Failed to remove repo from DB: {:?}", e);
                        send_reply(format!("❌ Ошибка удаления <b>{}</b>: {}", repo_str, e)).await;
                    }
                }
            } else {
                send_reply(
                    "❌ Неверный формат! Указывайте репозиторий как <code>owner/repo</code>, например:\n<code>/untrack tokio-rs/tokio</code>".to_string()
                ).await;
            }
        }
        Command::List => {
            match db.get_tracked_repos().await {
                Ok(repos) => {
                    if repos.is_empty() {
                        send_reply("📭 Список отслеживаемых репозиториев пуст.".to_string()).await;
                    } else {
                        let mut lines = Vec::new();
                        lines.push(format!("<b>Отслеживаемые репозитории ({})</b>:", repos.len()));
                        for r in repos {
                            let last_seen = r.last_seen_id.as_deref()
                                .or(r.last_seen_sha.as_deref())
                                .unwrap_or("нет данных");
                            lines.push(format!("• <b>{}</b> (последний релиз: <code>{}</code>)", r.full_name(), last_seen));
                        }
                        send_reply(lines.join("\n")).await;
                    }
                }
                Err(e) => {
                    error!("Failed to list repos from DB: {:?}", e);
                    send_reply(format!("❌ Ошибка получения списка: {}", e)).await;
                }
            }
        }
    }

    Ok(())
}
