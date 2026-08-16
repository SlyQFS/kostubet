//! Public user commands and interactive start onboarding.
//!
//! Handles `/start`, `/help`, `/admin` overview, repository suggestions (`/suggest`),
//! user submission status (`/mysuggestions`), APK catalog (`/apps`), and downloading (`/getapk`).

use crate::config::RepoConfig;
use crate::db::tags::ItemType;
use crate::db::Database;
use crate::dialogue::state::{DialogueState, SubmitApkState, SuggestData, SuggestState};
use crate::dialogue::BotDialogue;
use crate::services::render::{build_apk_post_data, render_post_keyboard, render_post_text};
use crate::strings::ACCESS_DENIED;
use anyhow::Result;
use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

pub async fn handle_start(bot: &Bot, msg: &Message) -> Result<()> {
    let chat_id = msg.chat.id;

    let kb = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("💡 Предложить репозиторий", "start:suggest"),
            InlineKeyboardButton::callback("📱 Загрузить APK", "start:submitapk"),
        ],
        vec![
            InlineKeyboardButton::callback("📋 Каталог приложений", "start:apps"),
            InlineKeyboardButton::callback("❓ Справка", "start:help"),
        ],
    ]);

    let text = "👋 <b>Добро пожаловать в Kostubet Bot!</b> 🚀\n\n\
        Бот отслеживает релизы инструментов на GitHub и публикует обновления приложений для Android (APK).\n\n\
        Выберите действие с помощью кнопок ниже или отправьте <code>/help</code> для вывода всех команд:";

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(kb)
        .await?;

    Ok(())
}

pub async fn handle_help(bot: &Bot, msg: &Message, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    send_help(bot, chat_id, sender_id, db).await
}

pub async fn send_help(bot: &Bot, chat_id: ChatId, sender_id: i64, db: &Database) -> Result<()> {
    let is_admin = db.admins().is_admin(sender_id).await.unwrap_or(false);

    let mut help_text = "<b>Kostubet GitHub & APK Watcher Bot 🚀</b>\n\n\
        <b>Публичные команды (в ЛС):</b>\n\
        • <code>/suggest [ссылка]</code> — Предложить GitHub-репозиторий (кнопочный мастер без аргументов)\n\
        • <code>/mysuggestions</code> — Статус ваших предложенных заявок\n\
        • <code>/submitapk</code> — Загрузить новое приложение / обновление APK\n\
        • <code>/apps</code> — Список опубликованных кастомных приложений\n\
        • <code>/getapk &lt;slug&gt;</code> — Получить APK приложения\n\
        • <code>/cancel</code> — Отменить текущий диалог\n\
        • <code>/help</code> — Эта справка\n"
        .to_string();

    if is_admin {
        help_text.push_str(
            "\n<b>Команды администратора:</b>\n\
            • <code>/admin</code> — Кнопочная панель управления (репозитории, теги, заявки, админы)\n\
            • <code>/track [ссылка] [#теги]</code> — Начать отслеживание (принимает ссылки GitHub)\n\
            • <code>/untrack owner/repo</code> — Прекратить отслеживание\n\
            • <code>/addtag owner/repo #tag</code> — Добавить тег инструменту\n\
            • <code>/removetag owner/repo #tag</code> — Удалить тег у инструмента\n\
            • <code>/list</code> — Список отслеживаемых инструментов\n\
            • <code>/tags</code> — Канонический список тегов\n\
            • <code>/pending</code> — Очередь заявок на модерацию\n\
            • <code>/admins</code> — Список администраторов\n\
            • <code>/test</code> — Отправить тестовую карточку релиза\n",
        );

        if db.admins().is_owner(sender_id).await.unwrap_or(false) {
            help_text.push_str(
                "\n<b>Команды владельца:</b>\n\
                • <code>/addadmin &lt;id&gt;</code> — Добавить администратора\n\
                • <code>/removeadmin &lt;id&gt;</code> — Удалить администратора\n\
                • <code>/debug</code> — Диагностика и состояние бота\n",
            );
        }
    }

    bot.send_message(chat_id, help_text)
        .parse_mode(ParseMode::Html)
        .await?;

    Ok(())
}

pub async fn handle_admin_panel(bot: &Bot, msg: &Message, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    if !db.admins().is_admin(sender_id).await? {
        bot.send_message(chat_id, ACCESS_DENIED)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    crate::handlers::callbacks::panel::send_root_panel(bot, chat_id, db).await
}

pub async fn handle_suggest(
    bot: &Bot,
    msg: &Message,
    args: &str,
    dialogue: &BotDialogue,
    db: &Database,
) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    let parts: Vec<&str> = args.split_whitespace().collect();

    // No argument: start the button-driven link flow.
    if parts.is_empty() {
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
        return Ok(());
    }

    // With an argument: parse, validate, show the confirmation card.
    let Some(repo) = RepoConfig::parse_ref(parts[0]) else {
        bot.send_message(
            chat_id,
            "❌ Неверный репозиторий! Отправьте ссылку вида\n\
            <code>https://github.com/owner/repo</code> или <code>owner/repo</code>\n\n\
            Например: <code>/suggest https://github.com/2dust/v2rayNG</code>",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    };

    // Inline tags right away when provided: /suggest owner/repo #tag1 #tag2
    let tags: Vec<String> = parts[1..]
        .iter()
        .map(|t| t.trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty())
        .collect();

    if !crate::dialogue::validate_new_suggestion(bot, chat_id, sender_id, &repo, db).await? {
        return Ok(());
    }

    let data = SuggestData {
        owner: repo.owner,
        name: repo.name,
        tags,
    };

    dialogue
        .update(DialogueState::Suggest(SuggestState::Confirm {
            data: Box::new(data.clone()),
        }))
        .await?;

    crate::dialogue::send_suggest_confirm(bot, chat_id, &data).await?;

    Ok(())
}

pub async fn handle_mysuggestions(bot: &Bot, msg: &Message, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    let suggestions = db.suggestions().get_user_suggestions(sender_id).await?;
    let custom_apps = db.custom_apps().get_user_versions(sender_id).await?;

    if suggestions.is_empty() && custom_apps.is_empty() {
        bot.send_message(chat_id, "📭 У вас пока нет созданных заявок.")
            .await?;
        return Ok(());
    }

    let mut lines = Vec::new();
    lines.push("📋 <b>Ваши последние заявки:</b>\n".to_string());

    if !suggestions.is_empty() {
        lines.push("<b>GitHub-репозитории:</b>".to_string());
        for s in suggestions {
            let status_badge = match s.status.as_str() {
                "approved" => "✅ Одобрено",
                "rejected" => "❌ Отклонено",
                _ => "⏳ На рассмотрении",
            };
            lines.push(format!("• <b>{}</b> — {}", s.full_name(), status_badge));
        }
        // Avoid a dangling blank line when the APK section below is empty.
        if !custom_apps.is_empty() {
            lines.push(String::new());
        }
    }

    if !custom_apps.is_empty() {
        lines.push("<b>APK-приложения:</b>".to_string());
        for (ver, app) in custom_apps {
            let status_badge = match ver.status.as_str() {
                "approved" => "✅ Одобрено",
                "rejected" => "❌ Отклонено",
                _ => "⏳ На рассмотрении",
            };
            lines.push(format!(
                "• <b>{}</b> (v{}) — {}",
                encode_text(&app.name),
                encode_text(&ver.version),
                status_badge
            ));
        }
    }

    bot.send_message(chat_id, lines.join("\n"))
        .parse_mode(ParseMode::Html)
        .await?;

    Ok(())
}

pub async fn handle_submitapk(
    bot: &Bot,
    msg: &Message,
    dialogue: &BotDialogue,
    db: &Database,
) -> Result<()> {
    let chat_id = msg.chat.id;
    let sender_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    start_submitapk(bot, chat_id, sender_id, dialogue, db).await
}

pub async fn start_submitapk(
    bot: &Bot,
    chat_id: ChatId,
    sender_id: i64,
    dialogue: &BotDialogue,
    db: &Database,
) -> Result<()> {
    let is_admin = db.admins().is_admin(sender_id).await.unwrap_or(false);

    if !is_admin {
        let pending_sugg = db
            .suggestions()
            .count_pending_for_user(sender_id)
            .await
            .unwrap_or(0);
        let pending_apk = db
            .custom_apps()
            .count_pending_versions_for_user(sender_id)
            .await
            .unwrap_or(0);
        if pending_sugg + pending_apk >= 3 {
            bot.send_message(
                chat_id,
                "⚠️ <b>Лимит заявок превышен!</b> У вас уже есть 3 активные заявки на рассмотрении. Пожалуйста, дождитесь их модерации.",
            )
            .parse_mode(ParseMode::Html)
            .await?;
            return Ok(());
        }
    }

    // An unfinished submission exists: ask instead of silently discarding it.
    let cur_state = dialogue.get().await?;
    if matches!(cur_state, Some(DialogueState::SubmitApk(_))) {
        let kb = InlineKeyboardMarkup::new(vec![
            vec![
                InlineKeyboardButton::callback("▶️ Продолжить текущую", "submitresume:continue"),
                InlineKeyboardButton::callback("🔄 Начать заново", "submitresume:restart"),
            ],
            vec![InlineKeyboardButton::callback(
                "❌ Отменить всё",
                "submitresume:cancel",
            )],
        ]);

        bot.send_message(
            chat_id,
            "⚠️ <b>У вас есть незавершенная заявка.</b>\nЧто сделать с ней?",
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(kb)
        .await?;
        return Ok(());
    }

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

    bot.send_message(
        chat_id,
        "📱 <b>Мастер публикации приложения / APK</b>\n\n\
        Выберите тип публикации:",
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(kb)
    .await?;

    Ok(())
}

pub async fn handle_apps(bot: &Bot, msg: &Message, db: &Database) -> Result<()> {
    send_apps_list(bot, msg.chat.id, db).await
}

pub async fn send_apps_list(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    crate::handlers::callbacks::panel::send_apps_page(bot, chat_id, db).await
}

pub async fn handle_getapk(bot: &Bot, msg: &Message, args: &str, db: &Database) -> Result<()> {
    let chat_id = msg.chat.id;
    let slug = args.trim();

    if slug.is_empty() {
        bot.send_message(
            chat_id,
            "❌ Укажите идентификатор приложения! Например:\n<code>/getapk v2rayng</code>\n\nСписок доступных приложений: <code>/apps</code>",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    let app = match db.custom_apps().get_app_by_slug(slug).await? {
        Some(a) => a,
        None => {
            bot.send_message(
                chat_id,
                format!("❌ Приложение с идентификатором <code>{}</code> не найдено.\nСписок: <code>/apps</code>", encode_text(slug)),
            )
            .parse_mode(ParseMode::Html)
            .await?;
            return Ok(());
        }
    };

    let ver = match db.custom_apps().get_current_version(app.id).await? {
        Some(v) => v,
        None => {
            bot.send_message(
                chat_id,
                "⚠️ Для этого приложения пока нет одобренных версий.",
            )
            .await?;
            return Ok(());
        }
    };

    let apk_files = db.custom_apps().get_apk_files(ver.id).await?;
    if apk_files.is_empty() {
        bot.send_message(chat_id, "⚠️ Файлы для скачивания не найдены.")
            .await?;
        return Ok(());
    }

    let apk_tuples: Vec<(i64, String)> = apk_files
        .into_iter()
        .map(|f| (f.id, f.variant_label))
        .collect();

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
        ver.changelog.clone(),
        ver.diff_url.clone(),
        ver.cover_image_file_id.clone(),
        tags,
        &apk_tuples,
    );

    let text = render_post_text(&post);
    let kb = render_post_keyboard(&post);

    let mut req = bot.send_message(chat_id, text).parse_mode(ParseMode::Html);

    if let Some(keyboard) = kb {
        req = req.reply_markup(keyboard);
    }

    req.await?;

    Ok(())
}
