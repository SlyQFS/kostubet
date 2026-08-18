//! Button-driven repository suggestion dialogue.
//!
//! Users send a GitHub link (or `owner/repo`), the bot parses it, shows a
//! confirmation card with inline buttons, optionally collects tags, and files
//! the suggestion into the existing moderation queue.

use crate::config::RepoConfig;
use crate::db::Database;
use crate::dialogue::state::{DialogueState, SuggestData, SuggestState};
use crate::dialogue::BotDialogue;
use anyhow::Result;
use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

/// Shared validation for a new suggestion: anti-spam limit + dedupe.
/// Sends an explanatory message and returns `false` when the suggestion
/// must not be filed.
pub async fn validate_new_suggestion(
    bot: &Bot,
    chat_id: ChatId,
    sender_id: i64,
    repo: &RepoConfig,
    db: &Database,
) -> Result<bool> {
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
            return Ok(false);
        }
    }

    if db.tools().get_tool(&repo.owner, &repo.name).await?.is_some() {
        bot.send_message(
            chat_id,
            format!(
                "ℹ️ Репозиторий <b>{}</b> уже отслеживается ботом.",
                repo.full_name()
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(false);
    }

    if let Some(existing) = db
        .suggestions()
        .find_pending_by_repo(&repo.owner, &repo.name)
        .await?
    {
        bot.send_message(
            chat_id,
            format!(
                "ℹ️ Репозиторий <b>{}</b> уже предложен (заявка #{}) и ждёт модерации.",
                repo.full_name(),
                existing.id
            ),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(false);
    }

    Ok(true)
}

/// Renders the confirmation card for a parsed repository.
pub fn suggest_confirm_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("✅ Отправить", "sugg_send"),
            InlineKeyboardButton::callback("🏷 Теги", "sugg_tags"),
        ],
        vec![
            InlineKeyboardButton::callback("📝 Описание", "sugg_desc"),
            InlineKeyboardButton::callback("❌ Отменить", "sugg_cancel"),
        ],
    ])
}

pub fn skip_or_cancel_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("⏩ Пропустить", "sugg_skip"),
        InlineKeyboardButton::callback("❌ Отменить", "sugg_cancel"),
    ]])
}

pub fn cancel_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("❌ Отменить", "sugg_cancel"),
    ]])
}

pub async fn send_suggest_confirm(
    bot: &Bot,
    chat_id: ChatId,
    data: &SuggestData,
) -> Result<()> {
    let tags_str = if data.tags.is_empty() {
        "нет".to_string()
    } else {
        data.tags
            .iter()
            .map(|t| format!("#{}", t))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let desc_str = match data.description.as_deref().map(str::trim) {
        Some(d) if !d.is_empty() => encode_text(d).into_owned(),
        _ => "не задано".to_string(),
    };

    bot.send_message(
        chat_id,
        format!(
            "💡 <b>Предложение репозитория</b>\n\n\
            📦 <code>{}/{}</code>\n\
            🔗 https://github.com/{}/{}\n\
            📝 Описание: <i>{}</i>\n\
            🏷 Теги: <code>{}</code>\n\n\
            Отправить заявку?",
            encode_text(&data.owner),
            encode_text(&data.name),
            encode_text(&data.owner),
            encode_text(&data.name),
            desc_str,
            encode_text(&tags_str),
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(suggest_confirm_keyboard())
    .await?;
    Ok(())
}

pub async fn handle_suggest_message(
    bot: Bot,
    msg: Message,
    dialogue: BotDialogue,
    state: SuggestState,
    db: Database,
) -> Result<()> {
    let chat_id = msg.chat.id;
    let text = msg.text().unwrap_or_default().trim().to_string();

    if text == "/cancel" || text == "/start" {
        dialogue.exit().await?;
        if text == "/cancel" {
            bot.send_message(chat_id, "❌ Предложение отменено.").await?;
        }
        return Ok(());
    }

    match state {
        SuggestState::WaitingLink => {
            let Some(repo) = RepoConfig::parse_ref(&text) else {
                bot.send_message(
                    chat_id,
                    "❌ Отправьте ссылку вида: <code>https://github.com/owner/example</code> или <code>owner/example</code>",
                )
                .reply_markup(cancel_keyboard())
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            };

            if !validate_new_suggestion(&bot, chat_id, msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0), &repo, &db).await? {
                dialogue.exit().await?;
                return Ok(());
            }

            let data = SuggestData {
                owner: repo.owner,
                name: repo.name,
                description: None,
                tags: Vec::new(),
            };

            dialogue
                .update(DialogueState::Suggest(SuggestState::Confirm {
                    data: Box::new(data.clone()),
                }))
                .await?;

            send_suggest_confirm(&bot, chat_id, &data).await?;
            Ok(())
        }
        SuggestState::Confirm { .. } => {
            bot.send_message(
                chat_id,
                "ℹ️ Нажмите <b>✅ Отправить</b>, измените описание/теги или нажмите <b>❌ Отменить</b>.",
            )
            .reply_markup(suggest_confirm_keyboard())
            .parse_mode(ParseMode::Html)
            .await?;
            Ok(())
        }
        SuggestState::WaitingDescription { mut data } => {
            if text != "/skip" && !text.is_empty() {
                if let Err(err) = crate::dialogue::validate_description(&text) {
                    bot.send_message(
                        chat_id,
                        format!("{}\n\nПовторите ввод или нажмите <b>Пропустить</b>.", err),
                    )
                    .reply_markup(skip_or_cancel_keyboard())
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }
                data.description = Some(text.clone());
            } else {
                data.description = None;
            }

            dialogue
                .update(DialogueState::Suggest(SuggestState::Confirm {
                    data: data.clone(),
                }))
                .await?;

            send_suggest_confirm(&bot, chat_id, &data).await?;
            Ok(())
        }
        SuggestState::WaitingTags { mut data } => {
            if text != "/skip" && !text.is_empty() {
                data.tags = text
                    .split_whitespace()
                    .map(|t| t.trim_start_matches('#').to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
            }

            dialogue
                .update(DialogueState::Suggest(SuggestState::Confirm {
                    data: data.clone(),
                }))
                .await?;

            send_suggest_confirm(&bot, chat_id, &data).await?;
            Ok(())
        }
    }
}
