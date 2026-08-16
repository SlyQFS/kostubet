//! APK application and release version submission dialogue flow.
//!
//! Guides users step-by-step through providing application names, versions,
//! optional titles/changelogs/diffs/screenshots, uploading APK files with CPU architecture
//! detection, and selecting tags before final review submission.

use crate::db::Database;
use crate::dialogue::state::{DialogueState, PendingApk, SubmitApkData, SubmitApkState};
use crate::dialogue::BotDialogue;
use crate::services::apk_variant::detect_variant;
use crate::services::render::{render_post_text, PostData};
use crate::strings::CANCEL_MESSAGE;
use anyhow::Result;
use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};

/// Transliterates a Cyrillic character into its Latin slug representation.
fn translit_char(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "y",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "c",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "sch",
        'ъ' => "",
        'ы' => "y",
        'ь' => "",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        _ => return None,
    })
}

/// Generates a URL-safe latin slug from an arbitrary (incl. Cyrillic) app name.
pub fn generate_slug(name: &str, user_id: i64) -> String {
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if let Some(t) = translit_char(c) {
            slug.push_str(t);
        } else if c == ' ' || c == '-' || c == '_' {
            slug.push('-');
        } else if c.is_ascii_alphanumeric() {
            slug.push(c);
        }
    }

    // Collapse repeated separators and trim edge separators.
    let mut collapsed = String::new();
    let mut last_dash = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_dash {
                collapsed.push('-');
            }
            last_dash = true;
        } else {
            collapsed.push(c);
            last_dash = false;
        }
    }
    let collapsed = collapsed.trim_matches('-').to_string();

    if collapsed.is_empty() {
        format!("app-{}", user_id)
    } else {
        collapsed
    }
}

pub fn variant_selection_keyboard() -> InlineKeyboardMarkup {
    let variants = ["universal", "arm64-v8a", "armeabi-v7a", "x86_64", "x86"];
    let mut rows = Vec::new();
    for v in variants {
        rows.push(vec![InlineKeyboardButton::callback(
            v,
            format!("variant_select:{}", v),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "❌ Отмена",
        "submit_confirm:cancel",
    )]);
    InlineKeyboardMarkup::new(rows)
}

#[tracing::instrument(skip(bot, dialogue, db))]
pub async fn handle_submit_message(
    bot: Bot,
    msg: Message,
    dialogue: BotDialogue,
    state: SubmitApkState,
    db: Database,
) -> Result<()> {
    let chat_id = msg.chat.id;
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    let text = msg.text().unwrap_or("").trim();

    // Global cancellation
    if text == "/cancel" {
        dialogue.exit().await?;
        bot.send_message(chat_id, CANCEL_MESSAGE).await?;
        return Ok(());
    }

    match state {
        SubmitApkState::ChoosingMode | SubmitApkState::ChoosingExistingApp => {
            bot.send_message(
                chat_id,
                "ℹ️ Пожалуйста, выберите опцию с помощью кнопок выше или отправьте <code>/cancel</code> для отмены.",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingName => {
            if text.is_empty() {
                bot.send_message(
                    chat_id,
                    "⚠️ Название не может быть пустым. Введите название приложения:",
                )
                .await?;
                return Ok(());
            }

            let name = text.to_string();
            let slug = generate_slug(&name, user_id);

            // Anti-merge guard: a different application with the same name
            // already exists — ask the user instead of silently attaching
            // a new version to someone else's app record.
            if let Some(existing) = db.custom_apps().get_app_by_slug(&slug).await? {
                dialogue
                    .update(DialogueState::SubmitApk(SubmitApkState::WaitingName))
                    .await?;

                let kb = InlineKeyboardMarkup::new(vec![
                    vec![InlineKeyboardButton::callback(
                        format!("🔄 Это обновление «{}»", existing.name),
                        format!("submit_app:{}", existing.slug),
                    )],
                    vec![InlineKeyboardButton::callback(
                        "✏️ Ввести другое название",
                        "submitslug:rename",
                    )],
                    vec![InlineKeyboardButton::callback(
                        "❌ Отмена",
                        "submit_confirm:cancel",
                    )],
                ]);

                bot.send_message(
                    chat_id,
                    format!(
                        "⚠️ Приложение с похожим названием уже существует в каталоге:\n\
                        • <b>{}</b> (<code>{}</code>)\n\n\
                        Если вы выпускаете новую версию именно этого приложения — выберите «Это обновление».\n\
                        Если это другое приложение — введите другое название.",
                        encode_text(&existing.name),
                        encode_text(&existing.slug),
                    ),
                )
                .parse_mode(ParseMode::Html)
                .reply_markup(kb)
                .await?;
                return Ok(());
            }

            let data = Box::new(SubmitApkData {
                is_new_app: true,
                app_id: None,
                slug,
                name,
                description: None,
                version: String::new(),
                title: None,
                changelog: None,
                diff_url: None,
                cover_image_file_id: None,
                apk_files: Vec::new(),
                tags: Vec::new(),
            });

            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::WaitingDescription {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                "📝 Введите описание приложения (что это за приложение — оно будет показываться в его карточках)\n\
                или отправьте <code>/skip</code>:",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingDescription { mut data } => {
            if text != "/skip" && !text.is_empty() {
                if let Err(err) = crate::dialogue::validate_description(text) {
                    bot.send_message(
                        chat_id,
                        format!("{}\n\nПовторите ввод или отправьте <code>/skip</code>.", err),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }
                data.description = Some(text.to_string());
            }

            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::WaitingVersion {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                "📦 Введите номер версии приложения (например, <code>1.0.0</code>):",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingVersion { mut data } => {
            if text.is_empty() {
                bot.send_message(chat_id, "⚠️ Версия не может быть пустой. Введите версию:")
                    .await?;
                return Ok(());
            }

            data.version = text.to_string();
            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::WaitingTitle {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                "📌 Введите заголовок карточки (краткий анонс) или отправьте <code>/skip</code>, чтобы пропустить:",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingTitle { mut data } => {
            if text != "/skip" && !text.is_empty() {
                data.title = Some(text.to_string());
            }

            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::WaitingChangelog {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                "📝 Введите список изменений (Changelog / описание) или отправьте <code>/skip</code>:",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingChangelog { mut data } => {
            if text != "/skip" && !text.is_empty() {
                data.changelog = Some(text.to_string());
            }

            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::WaitingDiffUrl {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                "🔗 Отправьте ссылку на изменения (GitHub Diff / Commit URL) или <code>/skip</code>:",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingDiffUrl { mut data } => {
            if text != "/skip" && !text.is_empty() {
                data.diff_url = Some(text.to_string());
            }

            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::WaitingCover {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                "🖼️ Отправьте обложку / скриншот приложения (фотографией) или <code>/skip</code>:",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingCover { mut data } => {
            if let Some(photos) = msg.photo() {
                if let Some(largest) = photos.iter().max_by_key(|p| p.width * p.height) {
                    data.cover_image_file_id = Some(largest.file.id.clone());
                }
            } else if text != "/skip" {
                bot.send_message(
                    chat_id,
                    "⚠️ Отправьте фотографию как обложку или команду <code>/skip</code>, чтобы продолжить без обложки:",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::WaitingApkFiles {
                    data,
                }))
                .await?;

            bot.send_message(
                chat_id,
                "📦 Отправьте один или несколько <b>.apk файлов</b> документом (до 2 ГБ — ограничение Telegram на документы).\n\
                Когда закончите загрузку всех файлов, отправьте команду <code>/done</code>:",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingApkFiles { mut data } => {
            if text == "/done" {
                if data.apk_files.is_empty() {
                    bot.send_message(
                        chat_id,
                        "⚠️ Вы еще не загрузили ни одного .apk файла! Отправьте хотя бы один .apk файл или <code>/cancel</code>.",
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }

                dialogue
                    .update(DialogueState::SubmitApk(SubmitApkState::WaitingTags {
                        data,
                    }))
                    .await?;

                bot.send_message(
                    chat_id,
                    "🏷️ Введите теги через пробел или запятую (например, <code>#vpn #android #free</code>) или отправьте <code>/skip</code>:",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            if let Some(doc) = msg.document() {
                let file_name = doc.file_name.clone().unwrap_or_default();
                if !file_name.to_lowercase().ends_with(".apk") {
                    bot.send_message(
                        chat_id,
                        "⚠️ Файл должен иметь расширение <b>.apk</b>. Пожалуйста, отправьте .apk файл документом:",
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }

                // Telegram document upload limit (2 GB). The bot never re-uploads
                // the APK itself — it stores the file_id and resends by it, which
                // has no size limit, so only the user-side upload limit applies.
                const MAX_APK_SIZE: u32 = 2 * 1024 * 1024 * 1024; // 2 GB
                if doc.file.size > MAX_APK_SIZE {
                    let size_gb = (doc.file.size as f64) / (1024.0 * 1024.0 * 1024.0);
                    bot.send_message(
                        chat_id,
                        format!(
                            "⚠️ <b>Размер файла превышает лимит 2 ГБ ({:.2} ГБ)!</b>\n\n\
                            Telegram не позволяет отправлять документы больше 2 ГБ.\n\
                            Пожалуйста, оптимизируйте размер APK или загрузите файл вручную.",
                            size_gb
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }

                let file_id = doc.file.id.clone();
                let file_unique_id = doc.file.unique_id.clone();
                let file_size = Some(doc.file.size as i64);

                let variant_opt = detect_variant(&file_name);
                if let Some(variant) = variant_opt {
                    data.apk_files.push(PendingApk {
                        variant: variant.to_string(),
                        file_id,
                        file_unique_id,
                        file_name: Some(file_name.clone()),
                        file_size,
                    });

                    let count = data.apk_files.len();
                    dialogue
                        .update(DialogueState::SubmitApk(SubmitApkState::WaitingApkFiles {
                            data,
                        }))
                        .await?;

                    bot.send_message(
                        chat_id,
                        format!(
                            "✅ Файл <code>{}</code> добавлен (архитектура: <b>{}</b>).\nВсего файлов: <b>{}</b>.\n\nОтправьте следующий .apk файл или команду <code>/done</code>.",
                            encode_text(&file_name),
                            variant,
                            count
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                } else {
                    // Ask variant manually
                    let pending_file = PendingApk {
                        variant: "unknown".to_string(),
                        file_id,
                        file_unique_id,
                        file_name: Some(file_name.clone()),
                        file_size,
                    };

                    dialogue
                        .update(DialogueState::SubmitApk(SubmitApkState::ResolvingVariant {
                            data,
                            pending_file,
                        }))
                        .await?;

                    bot.send_message(
                        chat_id,
                        format!(
                            "❓ Не удалось автоматически определить архитектуру для файла <code>{}</code>.\nВыберите архитектуру:",
                            encode_text(&file_name)
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .reply_markup(variant_selection_keyboard())
                    .await?;
                }
            } else {
                bot.send_message(
                    chat_id,
                    "⚠️ Отправьте <b>.apk файл</b> как документ (до 2 ГБ) или команду <code>/done</code>, когда все файлы загружены.",
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
        }
        SubmitApkState::ResolvingVariant { .. } => {
            bot.send_message(
                chat_id,
                "ℹ️ Пожалуйста, выберите архитектуру кнопкой выше или отправьте <code>/cancel</code> для отмены.",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        SubmitApkState::WaitingTags { mut data } => {
            if text != "/skip" && !text.is_empty() {
                let tags: Vec<String> = text
                    .split(&[' ', ','][..])
                    .map(|t| t.trim().trim_start_matches('#').to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
                data.tags = tags;
            }

            let post = PostData {
                title: format!("{} v{}", data.name, data.version),
                description: data.description.clone(),
                body: data.changelog.clone(),
                diff_url: data.diff_url.clone(),
                tags: data.tags.clone(),
                cover_image: data.cover_image_file_id.clone(),
                download_buttons: Vec::new(),
            };

            let preview_text = render_post_text(&post);
            let confirm_kb = InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("🚀 Отправить", "submit_confirm:send"),
                InlineKeyboardButton::callback("❌ Отмена", "submit_confirm:cancel"),
            ]]);

            dialogue
                .update(DialogueState::SubmitApk(SubmitApkState::Confirm { data }))
                .await?;

            bot.send_message(
                chat_id,
                format!(
                    "👀 <b>Предпросмотр карточки релиза:</b>\n\n{}\n\n<i>ℹ️ При публикации APK-файлы будут отправлены в чат сразу после карточки.</i>\n\n━━━━━━━━━━━━━━━\nВсё верно?",
                    preview_text
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(confirm_kb)
            .await?;
        }
        SubmitApkState::Confirm { .. } => {
            bot.send_message(
                chat_id,
                "ℹ️ Подтвердите отправку кнопкой <b>🚀 Отправить</b> или отмените кнопкой <b>❌ Отмена</b>.",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::generate_slug;

    #[test]
    fn test_slug_from_latin_name() {
        assert_eq!(generate_slug("V2RayNG", 1), "v2rayng");
        assert_eq!(generate_slug("Telegram X", 1), "telegram-x");
        assert_eq!(generate_slug("Some--App!! Name", 1), "some-app-name");
    }

    #[test]
    fn test_slug_transliteration() {
        assert_eq!(generate_slug("Телеграм", 1), "telegram");
        assert_eq!(generate_slug("Мой Календарь", 1), "moy-kalendar");
        assert_eq!(generate_slug("Ютуб", 1), "yutub");
    }

    #[test]
    fn test_slug_fallback_when_empty() {
        assert_eq!(generate_slug("!!!", 42), "app-42");
        assert_eq!(generate_slug("ъь", 7), "app-7");
    }

    #[test]
    fn test_slug_collapses_separators() {
        assert_eq!(generate_slug("  spaced   out  ", 1), "spaced-out");
        assert_eq!(generate_slug("under_score", 1), "under-score");
    }
}
