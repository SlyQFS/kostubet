//! Inline admin panel and paginated list views.
//!
//! Builds nested button menus over the `adm:*` callback namespace
//! (root panel -> repositories / tags / moderation queue / admins),
//! plus the public paginated APK catalog (`pub:apps:*`).

use crate::db::tags::ItemType;
use crate::db::Database;
use crate::dialogue::{AdminState, DialogueState};
use crate::dialogue::BotDialogue;
use crate::services::render::build_apk_post_data;
use crate::strings::ACCESS_DENIED;
use anyhow::Result;
use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{
    ChatId, InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode,
};

pub const REPOS_PAGE_SIZE: usize = 8;
pub const TAGS_PAGE_SIZE: usize = 10;
pub const PENDING_PAGE_SIZE: usize = 6;
pub const APPS_PAGE_SIZE: usize = 8;

fn btn(label: impl Into<String>, data: impl Into<String>) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(label.into(), data.into())
}

/// Builds a `⬅️ | page/total | ➡️` navigation row (empty when a single page).
fn nav_row(prefix: &str, page: usize, total_pages: usize) -> Vec<Vec<InlineKeyboardButton>> {
    if total_pages <= 1 {
        return Vec::new();
    }
    let mut row = Vec::new();
    if page > 0 {
        row.push(btn("⬅️", format!("{}:{}", prefix, page - 1)));
    }
    row.push(btn(format!("{}/{}", page + 1, total_pages), "adm:noop"));
    if page + 1 < total_pages {
        row.push(btn("➡️", format!("{}:{}", prefix, page + 1)));
    }
    vec![row]
}

fn total_pages(len: usize, per_page: usize) -> usize {
    len.div_ceil(per_page).max(1)
}

/// Updates the panel message in place; falls back to sending a new one.
async fn edit_or_send(
    bot: &Bot,
    q: &CallbackQuery,
    text: String,
    kb: InlineKeyboardMarkup,
) -> Result<()> {
    if let Some(msg) = &q.message {
        let edited = bot
            .edit_message_text(msg.chat().id, msg.id(), text.clone())
            .parse_mode(ParseMode::Html)
            .reply_markup(kb.clone())
            .await;
        if edited.is_ok() {
            return Ok(());
        }
    }
    let chat_id = q
        .message
        .as_ref()
        .map(|m| m.chat().id)
        .unwrap_or(ChatId(q.from.id.0 as i64));
    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(kb)
        .await?;
    Ok(())
}

async fn send_view(
    bot: &Bot,
    chat_id: ChatId,
    text: String,
    kb: Option<InlineKeyboardMarkup>,
) -> Result<()> {
    let mut req = bot.send_message(chat_id, text).parse_mode(ParseMode::Html);
    if let Some(kb) = kb {
        req = req.reply_markup(kb);
    }
    req.await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// View builders
// ---------------------------------------------------------------------------

async fn root_view(db: &Database) -> Result<(String, InlineKeyboardMarkup)> {
    let tool_count = db.tools().list_tools().await.map(|l| l.len()).unwrap_or(0);
    let app_count = db
        .custom_apps()
        .list_approved_apps()
        .await
        .map(|l| l.len())
        .unwrap_or(0);
    let pending_repo = db
        .suggestions()
        .get_pending_suggestions()
        .await
        .map(|l| l.len())
        .unwrap_or(0);
    let pending_apk = db
        .custom_apps()
        .get_pending_versions()
        .await
        .map(|l| l.len())
        .unwrap_or(0);

    let text = format!(
        "👑 <b>Панель Администратора</b>\n\n\
        📦 <b>Отслеживается репозиториев:</b> <code>{}</code>\n\
        📱 <b>Опубликовано APK-приложений:</b> <code>{}</code>\n\
        ⏳ <b>В очереди модерации:</b> <code>{}</code> (репозитории: {}, APK: {})",
        tool_count, app_count, pending_repo + pending_apk, pending_repo, pending_apk,
    );

    let kb = InlineKeyboardMarkup::new(vec![
        vec![
            btn("📺 Репозитории", "adm:repos:0"),
            btn("🏷 Теги", "adm:tags:0"),
        ],
        vec![
            btn(
                format!("📥 Заявки ({})", pending_repo + pending_apk),
                "adm:pending:0",
            ),
            btn("📱 Приложения", "adm:apps:0"),
        ],
        vec![btn("👥 Админы", "adm:admins"), btn("🔁 Обновить", "adm:root")],
        vec![btn("📜 Журнал действий", "adm:audit:0")],
    ]);

    Ok((text, kb))
}

async fn repos_page_view(
    db: &Database,
    page: usize,
) -> Result<(String, InlineKeyboardMarkup)> {
    let tools = db.tools().list_tools().await?;
    let pages = total_pages(tools.len(), REPOS_PAGE_SIZE);
    let page = page.min(pages - 1);
    let chunk = &tools[page * REPOS_PAGE_SIZE..(page * REPOS_PAGE_SIZE + REPOS_PAGE_SIZE).min(tools.len())];

    let mut text = format!(
        "📺 <b>Отслеживаемые репозитории</b> ({}, стр. {}/{})",
        tools.len(),
        page + 1,
        pages
    );
    if tools.is_empty() {
        text.push_str("\n\n📭 Пока ничего не отслеживается.");
    }

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for t in chunk {
        let tags = db
            .tags()
            .get_tags_for_item(ItemType::Tool, t.id)
            .await
            .unwrap_or_default();
        let tags_str = if tags.is_empty() {
            String::new()
        } else {
            format!(
                " {}",
                tags.iter()
                    .map(|tg| format!("#{}", tg.name))
                    .collect::<Vec<_>>()
                    .join("")
            )
        };
        text.push_str(&format!(
            "\n• <code>{}</code>{}",
            encode_text(&t.full_name()),
            tags_str
        ));
        rows.push(vec![btn(
            format!("📦 {}/{}", t.owner, t.repo),
            format!("adm:repo:{}", t.id),
        )]);
    }

    rows.extend(nav_row("adm:repos", page, pages));
    rows.push(vec![
        btn("➕ Добавить", "adm:repoadd"),
        btn("⬅️ В панель", "adm:root"),
    ]);

    Ok((text, InlineKeyboardMarkup::new(rows)))
}

async fn repo_detail_view(db: &Database, tool_id: i64) -> Result<(String, InlineKeyboardMarkup)> {
    let Some(tool) = db.tools().get_tool_by_id(tool_id).await? else {
        return Ok((
            "⚠️ Репозиторий не найден (возможно, уже удален).".to_string(),
            InlineKeyboardMarkup::new(vec![vec![btn("⬅️ К списку", "adm:repos:0")]]),
        ));
    };

    let tags = db
        .tags()
        .get_tags_for_item(ItemType::Tool, tool.id)
        .await
        .unwrap_or_default();
    let tags_str = if tags.is_empty() {
        "нет".to_string()
    } else {
        tags.iter()
            .map(|t| format!("#{}", t.name))
            .collect::<Vec<_>>()
            .join(" ")
    };

    let text = format!(
        "📦 <b>{}</b>\n\n\
        🔖 Последний релиз: <code>{}</code>\n\
        🏷 Теги: <code>{}</code>",
        encode_text(&tool.full_name()),
        encode_text(tool.last_release.as_deref().unwrap_or("нет релизов")),
        encode_text(&tags_str),
    );

    let kb = InlineKeyboardMarkup::new(vec![
        vec![
            btn("🏷 Теги", format!("adm:repotags:{}", tool.id)),
            btn("🗑 Убрать", format!("adm:repountrack:{}", tool.id)),
        ],
        vec![btn("⬅️ К списку", "adm:repos:0")],
    ]);

    Ok((text, kb))
}

async fn repo_untrack_confirm_view(
    db: &Database,
    tool_id: i64,
) -> Result<(String, InlineKeyboardMarkup)> {
    let Some(tool) = db.tools().get_tool_by_id(tool_id).await? else {
        return Ok((
            "⚠️ Репозиторий не найден.".to_string(),
            InlineKeyboardMarkup::new(vec![vec![btn("⬅️ К списку", "adm:repos:0")]]),
        ));
    };

    let text = format!(
        "🗑 Убрать <b>{}</b> из отслеживания?\n\nСтатистика релизов будет удалена, теги откреплены.",
        encode_text(&tool.full_name())
    );

    let kb = InlineKeyboardMarkup::new(vec![
        vec![
            btn("✅ Да, убрать", format!("adm:repountrackok:{}", tool.id)),
            btn("❌ Отмена", format!("adm:repo:{}", tool.id)),
        ],
    ]);

    Ok((text, kb))
}

async fn repo_tags_view(db: &Database, tool_id: i64) -> Result<(String, InlineKeyboardMarkup)> {
    let Some(tool) = db.tools().get_tool_by_id(tool_id).await? else {
        return Ok((
            "⚠️ Репозиторий не найден.".to_string(),
            InlineKeyboardMarkup::new(vec![vec![btn("⬅️ К списку", "adm:repos:0")]]),
        ));
    };

    let tags = db
        .tags()
        .get_tags_for_item(ItemType::Tool, tool.id)
        .await
        .unwrap_or_default();

    let mut text = format!("🏷 <b>Теги: {}</b>", encode_text(&tool.full_name()));
    if tags.is_empty() {
        text.push_str("\n\n📭 Теги не прикреплены.");
    }

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for t in tags {
        text.push_str(&format!("\n• #{}", encode_text(&t.name)));
        rows.push(vec![btn(
            format!("🗑 #{}", t.name),
            format!("adm:repotagdel:{}:{}", tool.id, t.id),
        )]);
    }

    rows.push(vec![btn(
        "➕ Добавить тег",
        format!("adm:repotagadd:{}", tool.id),
    )]);
    rows.push(vec![btn(
        "⬅️ Назад к репозиторию",
        format!("adm:repo:{}", tool.id),
    )]);

    Ok((text, InlineKeyboardMarkup::new(rows)))
}

async fn tags_page_view(db: &Database, page: usize) -> Result<(String, InlineKeyboardMarkup)> {
    let tags = db.tags().list_tags_with_usage().await?;
    let pages = total_pages(tags.len(), TAGS_PAGE_SIZE);
    let page = page.min(pages - 1);
    let chunk = &tags[page * TAGS_PAGE_SIZE..(page * TAGS_PAGE_SIZE + TAGS_PAGE_SIZE).min(tags.len())];

    let mut text = format!(
        "🏷 <b>Теги</b> ({}, стр. {}/{})",
        tags.len(),
        page + 1,
        pages
    );
    if tags.is_empty() {
        text.push_str("\n\n📭 Тегов пока нет.");
    }

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for (t, usage) in chunk {
        text.push_str(&format!("\n• #{} — используется: {}", encode_text(&t.name), usage));
        rows.push(vec![btn(
            format!("🏷 #{} ({})", t.name, usage),
            format!("adm:tag:{}", t.id),
        )]);
    }

    rows.extend(nav_row("adm:tags", page, pages));
    rows.push(vec![
        btn("➕ Новый тег", "adm:tagadd"),
        btn("⬅️ В панель", "adm:root"),
    ]);

    Ok((text, InlineKeyboardMarkup::new(rows)))
}

async fn tag_detail_view(db: &Database, tag_id: i64) -> Result<(String, InlineKeyboardMarkup)> {
    let tags = db.tags().list_tags().await?;
    let Some(tag) = tags.iter().find(|t| t.id == tag_id) else {
        return Ok((
            "⚠️ Тег не найден.".to_string(),
            InlineKeyboardMarkup::new(vec![vec![btn("⬅️ К тегам", "adm:tags:0")]]),
        ));
    };

    let items = db.tags().list_items_for_tag(tag_id).await?;
    let mut text = format!("🏷 <b>#{}</b>\n\nИспользуется в (<code>{}</code>):", encode_text(&tag.name), items.len());
    if items.is_empty() {
        text.push_str("\n📭 Нигде не используется.");
    }
    for (item_type, item_id) in items {
        let label = match item_type.as_str() {
            "tool" => db
                .tools()
                .get_tool_by_id(item_id)
                .await
                .ok()
                .flatten()
                .map(|t| format!("📦 {}", encode_text(&t.full_name())))
                .unwrap_or_else(|| "📦 ?".to_string()),
            _ => db
                .custom_apps()
                .get_app_by_id(item_id)
                .await
                .ok()
                .flatten()
                .map(|a| format!("📱 {}", encode_text(&a.name)))
                .unwrap_or_else(|| "📱 ?".to_string()),
        };
        text.push_str(&format!("\n• {}", label));
    }

    let kb = InlineKeyboardMarkup::new(vec![
        vec![btn("🗑 Удалить тег", format!("adm:tagdel:{}", tag.id))],
        vec![btn("⬅️ К тегам", "adm:tags:0")],
    ]);

    Ok((text, kb))
}

async fn tag_delete_confirm_view(db: &Database, tag_id: i64) -> Result<(String, InlineKeyboardMarkup)> {
    let tags = db.tags().list_tags().await?;
    let Some(tag) = tags.iter().find(|t| t.id == tag_id) else {
        return Ok((
            "⚠️ Тег не найден.".to_string(),
            InlineKeyboardMarkup::new(vec![vec![btn("⬅️ К тегам", "adm:tags:0")]]),
        ));
    };

    let text = format!(
        "🗑 Удалить тег <b>#{}</b>?\n\nОн будет откреплен от всех репозиториев и приложений.",
        encode_text(&tag.name)
    );

    let kb = InlineKeyboardMarkup::new(vec![vec![
        btn("✅ Да, удалить", format!("adm:tagdelok:{}", tag.id)),
        btn("❌ Отмена", format!("adm:tag:{}", tag.id)),
    ]]);

    Ok((text, kb))
}

/// Compact one-message moderation queue with per-item action buttons.
async fn pending_page_view(
    db: &Database,
    page: usize,
) -> Result<(String, InlineKeyboardMarkup)> {
    let suggestions = db.suggestions().get_pending_suggestions().await?;
    let pending_apps = db.custom_apps().get_pending_versions().await?;
    let total = suggestions.len() + pending_apps.len();

    if total == 0 {
        let text = "✅ Нет активных заявок на модерацию.".to_string();
        let kb = InlineKeyboardMarkup::new(vec![vec![btn("⬅️ В панель", "adm:root")]]);
        return Ok((text, kb));
    }

    // Interleave both queues into a single ordered list of render actions.
    enum Item {
        Sugg(crate::db::suggestions::SuggestionRecord),
        Apk(crate::db::custom_apps::CustomAppVersionRecord, crate::db::custom_apps::CustomAppRecord),
    }
    let mut items: Vec<Item> = Vec::with_capacity(total);
    for s in suggestions {
        items.push(Item::Sugg(s));
    }
    for (ver, app) in pending_apps {
        items.push(Item::Apk(ver, app));
    }

    let pages = total_pages(total, PENDING_PAGE_SIZE);
    let page = page.min(pages - 1);
    let chunk = &items[page * PENDING_PAGE_SIZE..(page * PENDING_PAGE_SIZE + PENDING_PAGE_SIZE).min(total)];

    let mut text = format!(
        "📋 <b>Очередь модерации</b> ({} заявок, стр. {}/{})",
        total, page + 1, pages
    );

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for item in chunk {
        match item {
            Item::Sugg(s) => {
                let author = match &s.username {
                    Some(u) => format!("@{}", encode_text(u)),
                    None => format!("<code>{}</code>", s.user_id),
                };
                text.push_str(&format!(
                    "\n\n💡 #{} <code>{}</code> — {} <code>[{}]</code>",
                    s.id,
                    encode_text(&s.full_name()),
                    author,
                    s.created_at
                ));
                rows.push(vec![
                    btn(format!("✅ #{}", s.id), format!("suggest_approve:{}", s.id)),
                    btn(format!("❌ #{}", s.id), format!("suggest_reject:{}", s.id)),
                ]);
            }
            Item::Apk(ver, app) => {
                text.push_str(&format!(
                    "\n\n📱 #{} <b>{}</b> v{} — <code>{}</code> <code>[{}]</code>",
                    ver.id,
                    encode_text(&app.name),
                    encode_text(&ver.version),
                    ver.submitted_by,
                    ver.created_at
                ));
                rows.push(vec![
                    btn(format!("✅ #{}", ver.id), format!("apk_approve:{}", ver.id)),
                    btn(format!("✏️ #{}", ver.id), format!("apk_edit:{}", ver.id)),
                    btn(format!("❌ #{}", ver.id), format!("apk_reject:{}", ver.id)),
                ]);
            }
        }
    }

    rows.extend(nav_row("adm:pending", page, pages));
    rows.push(vec![btn("🔁 Обновить", "adm:pending:0"), btn("⬅️ В панель", "adm:root")]);

    Ok((text, InlineKeyboardMarkup::new(rows)))
}

async fn admins_view(db: &Database, user_id: i64) -> Result<(String, InlineKeyboardMarkup)> {
    let admins = db.admins().list_admins().await?;
    let is_owner = db.admins().is_owner(user_id).await.unwrap_or(false);

    let mut text = format!("👥 <b>Администраторы</b> ({})", admins.len());
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();

    for a in admins {
        let badge = if a.is_owner { "👑 Владелец" } else { "👤 Админ" };
        text.push_str(&format!("\n• {} — {}", a.display_name(), badge));
        if is_owner && !a.is_owner {
            rows.push(vec![btn(
                format!("🗑 Убрать {}", a.display_name()),
                format!("adm:admdel:{}", a.telegram_id),
            )]);
        }
    }

    if is_owner {
        text.push_str("\n\n➕ Добавить администратора: <code>/addadmin &lt;id&gt;</code>");
    }

    rows.push(vec![btn("⬅️ В панель", "adm:root")]);

    Ok((text, InlineKeyboardMarkup::new(rows)))
}

/// Public paginated APK catalog (also backs the `/apps` command).
pub async fn apps_page_view(db: &Database, page: usize) -> Result<(String, InlineKeyboardMarkup)> {
    let apps = db.custom_apps().list_approved_apps().await?;
    let pages = total_pages(apps.len(), APPS_PAGE_SIZE);
    let page = page.min(pages - 1);
    let chunk = &apps[page * APPS_PAGE_SIZE..(page * APPS_PAGE_SIZE + APPS_PAGE_SIZE).min(apps.len())];

    let mut text = format!(
        "📱 <b>Опубликованные приложения</b> ({}, стр. {}/{})",
        apps.len(),
        page + 1,
        pages
    );
    if apps.is_empty() {
        text.push_str("\n\n📭 Опубликованных приложений пока нет.\nПредложить свое: <code>/submitapk</code>");
    }

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for app in chunk {
        let ver_str = db
            .custom_apps()
            .get_current_version(app.id)
            .await
            .ok()
            .flatten()
            .map(|v| format!(" (v{})", encode_text(&v.version)))
            .unwrap_or_default();
        rows.push(vec![btn(
            format!("📱 {}{}", app.name, ver_str),
            format!("appcard:{}", app.slug),
        )]);
    }

    rows.extend(nav_row("pub:apps", page, pages));

    Ok((text, InlineKeyboardMarkup::new(rows)))
}

/// Admin view of published custom apps with deletion controls.
async fn adm_apps_page_view(db: &Database, page: usize) -> Result<(String, InlineKeyboardMarkup)> {
    let apps = db.custom_apps().list_approved_apps().await?;
    let pages = total_pages(apps.len(), APPS_PAGE_SIZE);
    let page = page.min(pages - 1);
    let chunk = &apps[page * APPS_PAGE_SIZE..(page * APPS_PAGE_SIZE + APPS_PAGE_SIZE).min(apps.len())];

    let mut text = format!(
        "📱 <b>Опубликованные приложения</b> ({}, стр. {}/{})\n\nℹ️ Нажмите на приложение, чтобы удалить его.",
        apps.len(),
        page + 1,
        pages
    );
    if apps.is_empty() {
        text = "📱 <b>Опубликованные приложения</b>\n\n📭 Пока нет опубликованных приложений.".to_string();
    }

    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for app in chunk {
        let ver_str = db
            .custom_apps()
            .get_current_version(app.id)
            .await
            .ok()
            .flatten()
            .map(|v| format!(" (v{})", encode_text(&v.version)))
            .unwrap_or_default();
        text.push_str(&format!("\n• <b>{}</b>{}", encode_text(&app.name), ver_str));
        rows.push(vec![btn(
            format!("🗑 {}{}", app.name, ver_str),
            format!("adm:appdel:{}", app.id),
        )]);
    }

    rows.extend(nav_row("adm:apps", page, pages));
    rows.push(vec![btn("⬅️ В панель", "adm:root")]);

    Ok((text, InlineKeyboardMarkup::new(rows)))
}

async fn app_delete_confirm_view(
    db: &Database,
    app_id: i64,
) -> Result<(String, InlineKeyboardMarkup)> {
    let Some(app) = db.custom_apps().get_app_by_id(app_id).await? else {
        return Ok((
            "⚠️ Приложение не найдено.".to_string(),
            InlineKeyboardMarkup::new(vec![vec![btn("⬅️ К приложениям", "adm:apps:0")]]),
        ));
    };

    let text = format!(
        "🗑 Удалить приложение <b>{}</b> (<code>{}</code>)?\n\n\
        Будут удалены: все его версии, файлы APK, теги и карточки из супергруппы.\n\
        Это действие необратимо.",
        encode_text(&app.name),
        encode_text(&app.slug)
    );

    let kb = InlineKeyboardMarkup::new(vec![vec![
        btn("✅ Да, удалить", format!("adm:appdelok:{}", app.id)),
        btn("❌ Отмена", "adm:apps:0"),
    ]]);

    Ok((text, kb))
}

// ---------------------------------------------------------------------------
// Command entry points (send a fresh page-0 message)
// ---------------------------------------------------------------------------

/// Paginated audit trail of administrative actions.
async fn audit_page_view(db: &Database, page: usize) -> Result<(String, InlineKeyboardMarkup)> {
    const PER_PAGE: i64 = 8;
    let total = db.audit().count_actions().await.unwrap_or(0) as usize;
    let pages = total_pages(total, PER_PAGE as usize);
    let page = page.min(pages - 1);

    let actions = db
        .audit()
        .recent_actions(PER_PAGE, (page as i64) * PER_PAGE)
        .await
        .unwrap_or_default();

    let mut text = format!(
        "📜 <b>Журнал действий администраторов</b> ({}, стр. {}/{})",
        total, page + 1, pages
    );
    if actions.is_empty() {
        text.push_str("\n\n📭 Пока нет записей.");
    }
    for a in actions {
        text.push_str(&format!(
            "\n\n• <code>[{}]</code> <b>{}</b> — {} {}\n  👤 <code>{}</code>",
            a.created_at,
            encode_text(&a.action),
            encode_text(&a.target),
            "",
            a.admin_id
        ));
    }

    let mut rows = nav_row("adm:audit", page, pages);
    rows.push(vec![btn("⬅️ В панель", "adm:root")]);

    Ok((text, InlineKeyboardMarkup::new(rows)))
}

pub async fn send_root_panel(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    let (text, kb) = root_view(db).await?;
    send_view(bot, chat_id, text, Some(kb)).await
}

pub async fn send_repos_page(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    let (text, kb) = repos_page_view(db, 0).await?;
    send_view(bot, chat_id, text, Some(kb)).await
}

pub async fn send_tags_page(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    let (text, kb) = tags_page_view(db, 0).await?;
    send_view(bot, chat_id, text, Some(kb)).await
}

pub async fn send_pending_page(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    let (text, kb) = pending_page_view(db, 0).await?;
    send_view(bot, chat_id, text, Some(kb)).await
}

pub async fn send_apps_page(bot: &Bot, chat_id: ChatId, db: &Database) -> Result<()> {
    let (text, kb) = apps_page_view(db, 0).await?;
    send_view(bot, chat_id, text, Some(kb)).await
}

// ---------------------------------------------------------------------------
// Callback routing
// ---------------------------------------------------------------------------

fn parse_id(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

#[allow(clippy::too_many_lines)]
pub async fn handle_panel_callback(
    bot: &Bot,
    q: &CallbackQuery,
    rest: &str,
    dialogue: &BotDialogue,
    db: &Database,
    user_id: i64,
    target_chat_id: i64,
) -> Result<()> {
    if rest == "noop" {
        return Ok(());
    }

    if !db.admins().is_admin(user_id).await? {
        if let Some(msg) = &q.message {
            bot.send_message(msg.chat().id, ACCESS_DENIED)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        return Ok(());
    }

    // Root
    if rest == "root" {
        let (text, kb) = root_view(db).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    // Repositories
    if let Some(p) = rest.strip_prefix("repos:") {
        let page = p.parse::<usize>().unwrap_or(0);
        let (text, kb) = repos_page_view(db, page).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if rest == "repoadd" {
        dialogue
            .update(DialogueState::Admin(Box::new(AdminState::RepoLink)))
            .await?;
        if let Some(msg) = &q.message {
            bot.send_message(
                msg.chat().id,
                "📺 Отправьте ссылку на репозиторий:\n\
                <code>https://github.com/owner/repo</code> или <code>owner/repo</code>\n\n\
                Отмена: <code>/cancel</code>",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        return Ok(());
    }

    // Track mode choice after a repo link was parsed (post current release / silent).
    if let Some(mode) = rest.strip_prefix("trackmode:") {
        let cur_state = dialogue.get().await?;
        let Some(DialogueState::Admin(state)) = cur_state else {
            return Ok(());
        };
        let AdminState::RepoMode { owner, name } = *state else {
            return Ok(());
        };

        let silent = mode == "silent";
        dialogue
            .update(DialogueState::Admin(Box::new(AdminState::RepoTags {
                owner,
                name,
                silent,
            })))
            .await?;

        if let Some(msg) = &q.message {
            let mode_note = if silent {
                "🔇 Текущий релиз будет пропущен."
            } else {
                "📣 Текущий релиз будет опубликован при первом опросе."
            };
            let _ = bot
                .edit_message_text(
                    msg.chat().id,
                    msg.id(),
                    format!(
                        "🏷 Введите теги для репозитория через пробел (например: <code>rust network</code>)\n\
                        или отправьте <code>/done</code>, чтобы добавить без тегов.\n\n{}",
                        mode_note
                    ),
                )
                .parse_mode(ParseMode::Html)
                .await;
        }
        return Ok(());
    }

    if let Some(id_str) = rest.strip_prefix("repo:") {
        let Some(tool_id) = parse_id(id_str) else { return Ok(()) };
        let (text, kb) = repo_detail_view(db, tool_id).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("repountrack:") {
        let Some(tool_id) = parse_id(id_str) else { return Ok(()) };
        let (text, kb) = repo_untrack_confirm_view(db, tool_id).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("repountrackok:") {
        let Some(tool_id) = parse_id(id_str) else { return Ok(()) };
        if let Some(tool) = db.tools().get_tool_by_id(tool_id).await? {
            let _ = db.tools().remove_tool(&tool.owner, &tool.repo).await?;
            let _ = db
                .audit()
                .log_action(user_id, "удалил репозиторий", &tool.full_name())
                .await;
        }
        let (text, kb) = repos_page_view(db, 0).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("repotags:") {
        let Some(tool_id) = parse_id(id_str) else { return Ok(()) };
        let (text, kb) = repo_tags_view(db, tool_id).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("repotagadd:") {
        let Some(tool_id) = parse_id(id_str) else { return Ok(()) };
        let Some(tool) = db.tools().get_tool_by_id(tool_id).await? else {
            return Ok(());
        };
        dialogue
            .update(DialogueState::Admin(Box::new(AdminState::ItemTag {
                item_type: ItemType::Tool.to_string(),
                item_id: tool.id,
                item_label: tool.full_name(),
            })))
            .await?;
        if let Some(msg) = &q.message {
            bot.send_message(
                msg.chat().id,
                "🏷 Введите название тега (например: <code>vpn</code>):\nОтмена: <code>/cancel</code>",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        return Ok(());
    }

    if let Some(ids) = rest.strip_prefix("repotagdel:") {
        let mut parts = ids.split(':');
        let (Some(tool_id), Some(tag_id)) = (parts.next().and_then(parse_id), parts.next().and_then(parse_id))
        else {
            return Ok(());
        };
        let _ = db.tags().detach_tag(ItemType::Tool, tool_id, tag_id).await?;
        let (text, kb) = repo_tags_view(db, tool_id).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    // Tags
    if let Some(p) = rest.strip_prefix("tags:") {
        let page = p.parse::<usize>().unwrap_or(0);
        let (text, kb) = tags_page_view(db, page).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if rest == "tagadd" {
        dialogue
            .update(DialogueState::Admin(Box::new(AdminState::NewTag)))
            .await?;
        if let Some(msg) = &q.message {
            bot.send_message(
                msg.chat().id,
                "🏷 Введите название нового тега (например: <code>android</code>):\nОтмена: <code>/cancel</code>",
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        return Ok(());
    }

    if let Some(id_str) = rest.strip_prefix("tag:") {
        let Some(tag_id) = parse_id(id_str) else { return Ok(()) };
        let (text, kb) = tag_detail_view(db, tag_id).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("tagdel:") {
        let Some(tag_id) = parse_id(id_str) else { return Ok(()) };
        let (text, kb) = tag_delete_confirm_view(db, tag_id).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("tagdelok:") {
        let Some(tag_id) = parse_id(id_str) else { return Ok(()) };
        let tag_name = db
            .tags()
            .list_tags()
            .await
            .ok()
            .and_then(|tags| tags.into_iter().find(|t| t.id == tag_id))
            .map(|t| t.name);
        let _ = db.tags().remove_tag_by_id(tag_id).await?;
        if let Some(name) = tag_name {
            let _ = db
                .audit()
                .log_action(user_id, "удалил тег", &format!("#{}", name))
                .await;
        }
        let (text, kb) = tags_page_view(db, 0).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    // Moderation queue
    if let Some(p) = rest.strip_prefix("pending:") {
        let page = p.parse::<usize>().unwrap_or(0);
        let (text, kb) = pending_page_view(db, page).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    // Published apps management (deletion)
    if let Some(p) = rest.strip_prefix("apps:") {
        let page = p.parse::<usize>().unwrap_or(0);
        let (text, kb) = adm_apps_page_view(db, page).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("appdel:") {
        let Some(app_id) = parse_id(id_str) else { return Ok(()) };
        let (text, kb) = app_delete_confirm_view(db, app_id).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("appdelok:") {
        let Some(app_id) = parse_id(id_str) else { return Ok(()) };

        // Remove the published channel/group posts first (best-effort).
        let mut removed_posts = 0usize;
        if target_chat_id != 0 {
            for msg_id in db
                .custom_apps()
                .get_published_message_ids_for_app(app_id)
                .await
                .unwrap_or_default()
            {
                if bot
                    .delete_message(ChatId(target_chat_id), MessageId(msg_id as i32))
                    .await
                    .is_ok()
                {
                    removed_posts += 1;
                }
            }
        }

        let app_name = db
            .custom_apps()
            .get_app_by_id(app_id)
            .await
            .ok()
            .flatten()
            .map(|a| a.name);
        let deleted = db.custom_apps().delete_app(app_id).await.unwrap_or(false);
        if deleted {
            let _ = db
                .audit()
                .log_action(
                    user_id,
                    "удалил приложение",
                    &format!("{} (постов удалено: {})", app_name.unwrap_or_default(), removed_posts),
                )
                .await;
        }

        let (mut text, kb) = adm_apps_page_view(db, 0).await?;
        if deleted {
            text = format!(
                "✅ Приложение удалено. Карточек удалено из супергруппы: {}.\n\n{}",
                removed_posts, text
            );
        }
        return edit_or_send(bot, q, text, kb).await;
    }

    // Audit trail
    if let Some(p) = rest.strip_prefix("audit:") {
        let page = p.parse::<usize>().unwrap_or(0);
        let (text, kb) = audit_page_view(db, page).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    // Admins
    if rest == "admins" {
        let (text, kb) = admins_view(db, user_id).await?;
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("admdel:") {
        let Some(admin_id) = parse_id(id_str) else { return Ok(()) };
        if !db.admins().is_owner(user_id).await? {
            if let Some(msg) = &q.message {
                bot.send_message(msg.chat().id, ACCESS_DENIED)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            return Ok(());
        }
        let text = format!(
            "🗑 Убрать администратора <code>{}</code>?",
            admin_id
        );
        let kb = InlineKeyboardMarkup::new(vec![vec![
            btn("✅ Да, убрать", format!("adm:admdelok:{}", admin_id)),
            btn("❌ Отмена", "adm:admins"),
        ]]);
        return edit_or_send(bot, q, text, kb).await;
    }

    if let Some(id_str) = rest.strip_prefix("admdelok:") {
        let Some(admin_id) = parse_id(id_str) else { return Ok(()) };
        if !db.admins().is_owner(user_id).await? {
            if let Some(msg) = &q.message {
                bot.send_message(msg.chat().id, ACCESS_DENIED)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            return Ok(());
        }
        let removed = db.admins().remove_admin(admin_id).await.unwrap_or(false);
        let (text, kb) = admins_view(db, user_id).await?;
        let text = if removed {
            format!("✅ Администратор <code>{}</code> удален.\n\n{}", admin_id, text)
        } else {
            format!("ℹ️ Пользователь <code>{}</code> не найден среди админов.\n\n{}", admin_id, text)
        };
        return edit_or_send(bot, q, text, kb).await;
    }

    Ok(())
}

/// Public: pagination inside the `/apps` catalog message.
pub async fn handle_apps_page_callback(
    bot: &Bot,
    q: &CallbackQuery,
    page: usize,
    db: &Database,
) -> Result<()> {
    let (text, kb) = apps_page_view(db, page).await?;
    edit_or_send(bot, q, text, kb).await
}

/// Public: renders a full app card as a new message (opened from `/apps`).
/// Uses `send_post`, so the user-uploaded cover photo and the download
/// keyboard are included, exactly like in the published supergroup card.
pub async fn handle_appcard_callback(
    bot: &Bot,
    q: &CallbackQuery,
    slug: &str,
    db: &Database,
) -> Result<()> {
    let chat_id = q
        .message
        .as_ref()
        .map(|m| m.chat().id)
        .unwrap_or(ChatId(q.from.id.0 as i64));

    let Some(app) = db.custom_apps().get_app_by_slug(slug).await? else {
        let _ = bot
            .answer_callback_query(q.id.clone())
            .text("⚠️ Приложение не найдено.")
            .await;
        return Ok(());
    };

    let Some(ver) = db.custom_apps().get_current_version(app.id).await? else {
        let _ = bot
            .answer_callback_query(q.id.clone())
            .text("⚠️ Нет одобренных версий.")
            .await;
        return Ok(());
    };

    let apk_files = db.custom_apps().get_apk_files(ver.id).await?;
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

    crate::services::render::send_post(bot, chat_id.0, None, &post).await?;
    Ok(())
}
