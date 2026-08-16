//! Common response strings and emoji dictionary.
//!
//! Emojis used across the bot:
//! - 🆕 New releases, updates, initial announcements
//! - 📦 GitHub repositories, APK binaries, packages
//! - 📱 Android applications, mobile versions
//! - 🏷️ Tags and categorizations (#tag)
//! - 👑 Bot owners, master controls
//! - 👤 Bot administrators, user profiles
//! - 💡 User repository suggestions
//! - ⏳ Pending moderation queue
//! - 📝 Changelogs, descriptions, release notes
//! - 🔗 External URLs, diff URLs, links
//! - ⬇️ Download links and buttons
//! - 🛠️ Debug diagnostics, technical maintenance
//! - ❓ Help, guide, interactive queries
//! - 🚀 Publishing, submitting, launching
//! - ✅ Successful operations, approvals
//! - ❌ Errors, rejections, cancellations
//! - ⚠️ Warnings, validation alerts, limits

pub const ACCESS_DENIED: &str = "⛔ Доступ запрещен.";
pub const OWNER_ONLY_ADD: &str = "⛔ Только <b>Владелец бота</b> может добавлять администраторов.";
pub const OWNER_ONLY_REMOVE: &str = "⛔ Только <b>Владелец бота</b> может удалять администраторов.";
pub const GROUP_NOTICE_HTML: &str =
    "ℹ️ Бот работает <b>только в личных сообщениях</b>. Пожалуйста, напишите боту в ЛС.";
pub const CANCEL_MESSAGE: &str = "❌ Процесс отменен.";
