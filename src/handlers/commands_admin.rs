//! Admin and owner command handlers.
//!
//! Handles bot administration commands including tracking tools, tag management,
//! admin permission control, moderation queue inspection, debug diagnostics, and test post delivery.

use crate::config::RepoConfig;
use crate::db::tags::ItemType;
use crate::db::Database;
use crate::services::render::{DownloadTarget, PostData};
use crate::strings::{ACCESS_DENIED, OWNER_ONLY_ADD, OWNER_ONLY_REMOVE};
use anyhow::Result;
use teloxide::prelude::*;
use teloxide::types::{ChatAction, ChatId, ParseMode};
use tracing::info;

pub async fn handle_addadmin(bot: &Bot, msg: &Message, args: &str, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_owner(sender_id).await? {
        bot.send_message(chat_id, OWNER_ONLY_ADD)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    let target_id_str = args.trim();
    let target_id: i64 = match target_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            bot.send_message(
                chat_id,
                "❌ Неверный формат ID! Использование:\n<code>/addadmin 123456789</code>",
            )
            .parse_mode(ParseMode::Html)
            .await?;
            return Ok(());
        }
    };

    match db
        .admins()
        .add_admin(target_id, None, Some(sender_id), false)
        .await
    {
        Ok(true) => {
            bot.send_message(
                chat_id,
                format!(
                    "✅ Пользователь <code>{}</code> успешно добавлен в список администраторов!",
                    target_id
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Ok(false) => {
            bot.send_message(
                chat_id,
                format!(
                    "ℹ️ Пользователь <code>{}</code> уже является администратором.",
                    target_id
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Err(e) => {
            bot.send_message(
                chat_id,
                format!("❌ Ошибка при добавлении администратора: {}", e),
            )
            .await?;
        }
    }

    Ok(())
}

pub async fn handle_removeadmin(bot: &Bot, msg: &Message, args: &str, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_owner(sender_id).await? {
        bot.send_message(chat_id, OWNER_ONLY_REMOVE)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    let target_id_str = args.trim();
    let target_id: i64 = match target_id_str.parse() {
        Ok(id) => id,
        Err(_) => {
            bot.send_message(
                chat_id,
                "❌ Неверный формат ID! Использование:\n<code>/removeadmin 123456789</code>",
            )
            .parse_mode(ParseMode::Html)
            .await?;
            return Ok(());
        }
    };

    match db.admins().remove_admin(target_id).await {
        Ok(true) => {
            bot.send_message(
                chat_id,
                format!(
                    "🗑️ Администратор <code>{}</code> успешно удален.",
                    target_id
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Ok(false) => {
            bot.send_message(
                chat_id,
                format!(
                    "ℹ️ Пользователь <code>{}</code> не найден среди администраторов.",
                    target_id
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Err(e) => {
            bot.send_message(
                chat_id,
                format!("❌ Ошибка при удалении администратора: {}", e),
            )
            .await?;
        }
    }

    Ok(())
}

pub async fn handle_admins(bot: &Bot, msg: &Message, db: &Database) -> Result<()> {
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(msg.chat.id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    send_admins_list(bot, msg.chat.id, db).await
}

pub async fn send_admins_list(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    let admins = db.admins().list_admins().await?;
    let mut lines = Vec::new();
    lines.push(format!(
        "👑 <b>Список администраторов бота ({})</b>:\n",
        admins.len()
    ));

    for a in admins {
        let badge = if a.is_owner {
            "👑 Владелец"
        } else {
            "👤 Админ"
        };
        lines.push(format!("• {} — {}", a.display_name(), badge));
    }

    bot.send_message(chat_id, lines.join("\n"))
        .parse_mode(ParseMode::Html)
        .await?;

    Ok(())
}

pub async fn handle_track(bot: &Bot, msg: &Message, args: &str, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(chat_id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        bot.send_message(
            chat_id,
            "❌ Неверный формат! Использование:\n<code>/track owner/repo [#tag1 #tag2]</code>",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    let repo_str = parts[0];
    let Some(repo) = RepoConfig::parse_ref(repo_str) else {
        bot.send_message(
            chat_id,
            "❌ Неверный репозиторий! Укажите в формате <code>owner/repo</code> (например: <code>/track tokio-rs/tokio</code>)",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    };

    let tags: Vec<String> = parts[1..]
        .iter()
        .map(|t| t.trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty())
        .collect();

    match db
        .tools()
        .add_tool(&repo.owner, &repo.name, sender_id)
        .await
    {
        Ok(tool_id) => {
            if !tags.is_empty() {
                let _ = db
                    .tags()
                    .set_tags_for_item(ItemType::Tool, tool_id, &tags)
                    .await;
            }

            let tags_str = if tags.is_empty() {
                "без тегов".to_string()
            } else {
                tags.iter()
                    .map(|t| format!("#{}", t))
                    .collect::<Vec<_>>()
                    .join(" ")
            };

            info!(
                "Started tracking tool: {} with tags [{}]",
                repo.full_name(),
                tags_str
            );
            bot.send_message(
                chat_id,
                format!(
                    "✅ Репозиторий <b>{}</b> добавлен в отслеживание!\nТеги: <code>{}</code>",
                    repo.full_name(),
                    tags_str
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Err(e) => {
            bot.send_message(
                chat_id,
                format!("❌ Ошибка при добавлении инструмента: {}", e),
            )
            .await?;
        }
    }

    Ok(())
}

pub async fn handle_untrack(bot: &Bot, msg: &Message, args: &str, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(chat_id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    let repo_str = args.trim();
    let Some(repo) = RepoConfig::parse_ref(repo_str) else {
        bot.send_message(
            chat_id,
            "❌ Неверный формат! Использование:\n<code>/untrack owner/repo</code>",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    };

    match db.tools().remove_tool(&repo.owner, &repo.name).await {
        Ok(true) => {
            info!("Stopped tracking tool: {}", repo.full_name());
            bot.send_message(
                chat_id,
                format!(
                    "🗑️ Репозиторий <b>{}</b> удален из отслеживания.",
                    repo.full_name()
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Ok(false) => {
            bot.send_message(
                chat_id,
                format!(
                    "ℹ️ Репозиторий <b>{}</b> не был в списке.",
                    repo.full_name()
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Err(e) => {
            bot.send_message(chat_id, format!("❌ Ошибка при удалении: {}", e))
                .await?;
        }
    }

    Ok(())
}

pub async fn handle_addtag(bot: &Bot, msg: &Message, args: &str, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(chat_id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        bot.send_message(
            chat_id,
            "❌ Использование:\n<code>/addtag owner/repo #tag</code>",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    let Some(repo) = RepoConfig::parse_ref(parts[0]) else {
        bot.send_message(chat_id, "❌ Неверный формат репозитория owner/repo.")
            .await?;
        return Ok(());
    };

    let Some(tool) = db.tools().get_tool(&repo.owner, &repo.name).await? else {
        bot.send_message(
            chat_id,
            format!("❌ Репозиторий {} не отслеживается.", repo.full_name()),
        )
        .await?;
        return Ok(());
    };

    let tag_name = parts[1].trim_start_matches('#');
    let tag_id = db.tags().get_or_create_tag(tag_name).await?;
    db.tags()
        .attach_tag(ItemType::Tool, tool.id, tag_id)
        .await?;

    bot.send_message(
        chat_id,
        format!(
            "✅ Тег <b>#{}</b> добавлен к <b>{}</b>",
            tag_name,
            repo.full_name()
        ),
    )
    .parse_mode(ParseMode::Html)
    .await?;

    Ok(())
}

pub async fn handle_removetag(bot: &Bot, msg: &Message, args: &str, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(chat_id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 2 {
        bot.send_message(
            chat_id,
            "❌ Использование:\n<code>/removetag owner/repo #tag</code>",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    let Some(repo) = RepoConfig::parse_ref(parts[0]) else {
        bot.send_message(chat_id, "❌ Неверный формат репозитория owner/repo.")
            .await?;
        return Ok(());
    };

    let Some(tool) = db.tools().get_tool(&repo.owner, &repo.name).await? else {
        bot.send_message(
            chat_id,
            format!("❌ Репозиторий {} не отслеживается.", repo.full_name()),
        )
        .await?;
        return Ok(());
    };

    let tag_name = parts[1].trim_start_matches('#');
    let removed = db
        .tags()
        .detach_tag_by_name(ItemType::Tool, tool.id, tag_name)
        .await?;

    if removed {
        bot.send_message(
            chat_id,
            format!(
                "🗑️ Тег <b>#{}</b> удален у <b>{}</b>",
                tag_name,
                repo.full_name()
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    } else {
        bot.send_message(
            chat_id,
            format!(
                "ℹ️ Тег <b>#{}</b> не был прикреплен к <b>{}</b>",
                tag_name,
                repo.full_name()
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
    }

    Ok(())
}

pub async fn handle_list(bot: &Bot, msg: &Message, db: &Database) -> Result<()> {
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(msg.chat.id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    send_tools_list(bot, msg.chat.id, db).await
}

pub async fn send_tools_list(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    crate::handlers::callbacks::panel::send_repos_page(bot, chat_id, db).await
}

pub async fn handle_tags(bot: &Bot, msg: &Message, db: &Database) -> Result<()> {
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(msg.chat.id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    send_tags_list(bot, msg.chat.id, db).await
}

pub async fn send_tags_list(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    crate::handlers::callbacks::panel::send_tags_page(bot, chat_id, db).await
}

pub async fn handle_pending(bot: &Bot, msg: &Message, db: &Database) -> Result<()> {
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(msg.chat.id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    send_pending(bot, msg.chat.id, db).await
}

pub async fn send_pending(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    // Compact one-message queue with inline action buttons and pagination
    // (replaces the old N+1 message flooding).
    crate::handlers::callbacks::panel::send_pending_page(bot, chat_id, db).await
}

pub async fn handle_debug(
    bot: &Bot,
    msg: &Message,
    db: &Database,
    target_chat_id: i64,
    target_thread_id: Option<i64>,
) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_owner(sender_id).await? {
        bot.send_message(chat_id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    let tool_count = db.tools().list_tools().await.map(|l| l.len()).unwrap_or(0);
    let app_count = db
        .custom_apps()
        .list_approved_apps()
        .await
        .map(|l| l.len())
        .unwrap_or(0);
    let admin_count = db
        .admins()
        .list_admins()
        .await
        .map(|l| l.len())
        .unwrap_or(0);
    let tag_count = db.tags().list_tags().await.map(|l| l.len()).unwrap_or(0);

    let text = format!(
        "🛠️ <b>Техническая диагностика бота</b>\n\n\
        👑 <b>Owner ID:</b> <code>{}</code>\n\
        🎯 <b>Целевой Chat ID:</b> <code>{}</code>\n\
        📌 <b>Topic Thread ID:</b> <code>{}</code>\n\n\
        📊 <b>Состояние базы данных:</b>\n\
        • Администраторов: <code>{}</code>\n\
        • Отслеживаемых инструментов: <code>{}</code>\n\
        • Опубликованных APK: <code>{}</code>\n\
        • Сохраненных тегов: <code>{}</code>\n\
        • Режим журнала: <code>WAL (Write-Ahead Logging)</code>",
        sender_id,
        target_chat_id,
        target_thread_id
            .map(|t| t.to_string())
            .unwrap_or_else(|| "Не задан".to_string()),
        admin_count,
        tool_count,
        app_count,
        tag_count,
    );

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .await?;

    Ok(())
}

pub async fn handle_test(
    bot: &Bot,
    msg: &Message,
    db: &Database,
    target_chat_id: i64,
    target_thread_id: Option<i64>,
) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(chat_id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    if target_chat_id == 0 {
        bot.send_message(
            chat_id,
            "⚠️ <b>TELEGRAM_CHAT_ID</b> равен 0! Укажите TELEGRAM_CHAT_ID в .env или config.toml.",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;

    let sample_post = PostData {
        title: "Tokio v1.42.0".to_string(),
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
            ```"
            .to_string(),
        ),
        diff_url: Some("https://github.com/tokio-rs/tokio/releases/tag/tokio-1.42.0".to_string()),
        tags: vec![
            "async".to_string(),
            "rust".to_string(),
            "network".to_string(),
        ],
        cover_image: None,
        download_buttons: vec![(
            "⬇️ Скачать (universal)".to_string(),
            DownloadTarget::Url("https://github.com/tokio-rs/tokio".to_string()),
        )],
    };

    match crate::services::render::send_post(bot, target_chat_id, target_thread_id, &sample_post)
        .await
    {
        Ok(_) => {
            let dest = match target_thread_id {
                Some(tid) => format!(
                    "Группа <code>{}</code> (Топик <code>{}</code>)",
                    target_chat_id, tid
                ),
                None => format!("Группа <code>{}</code>", target_chat_id),
            };
            bot.send_message(
                chat_id,
                format!(
                    "✅ <b>Тестовая карточка успешно отправлена!</b>\nНазначение: {}",
                    dest
                ),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Err(e) => {
            bot.send_message(chat_id, format!("❌ Ошибка отправки карточки: {}", e))
                .await?;
        }
    }

    Ok(())
}
