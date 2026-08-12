# Kostubet GitHub Watcher Telegram Bot 🚀

High-performance, lightweight Rust application that monitors GitHub repositories for releases, tags, or commits, and posts beautifully formatted HTML update cards into a specific topic thread within a Telegram supergroup forum.

---

## ✨ Features

- **Zero-Config Env Setup**: Run directly with environment variables (`TRACKED_REPOS="owner/repo1,owner/repo2"`) without editing config files.
- **Ultra-Lightweight Container**: Multi-stage Alpine build (~20MB total container image size).
- **Beautiful HTML Cards**: Modern visual card layout with badges (🚀 `[RELEASE]`, 🏷️ `[TAG]`, 📝 `[COMMIT]`), emojis, and Telegram `<blockquote expandable>` for clean changelogs.
- **Local Dry-Run Mode**: Test fetching and preview output cards directly in your terminal (`--dry-run` or `DRY_RUN=true`) without sending Telegram messages or requiring bot credentials.
- **Forum Topic Support**: Posts directly into a Telegram supergroup topic thread (`TELEGRAM_ARCHIVE_THREAD_ID`).
- **ETag Zero-Quota Polling**: Uses conditional `If-None-Match` HTTP requests. Unchanged repos return `304 Not Modified` and consume **zero** rate-limit quota.
- **SQLite State Persistence**: Saves last seen ID, SHA, and ETag. State updates **only after** confirmed Telegram delivery.
- **Interactive Commands**: Manage tracked repositories dynamically via Telegram (`/track`, `/untrack`, `/list`, `/help`).

---

## 💻 Quick Start & Fast Setup

### 1. Environment-Only Fast Launch (No config files!)

Create a `.env` file from `.env.example`:

```bash
cp .env.example .env
```

Edit `.env`:

```env
TELEGRAM_BOT_TOKEN=123456789:ABCdefGHIjklMNOpqrsTUVwxyZ
TELEGRAM_CHAT_ID=-1001234567890
TELEGRAM_ARCHIVE_THREAD_ID=42
TRACKED_REPOS=rust-lang/rust,tokio-rs/tokio,teloxide/teloxide
```

Start immediately with Docker Compose:

```bash
docker compose up -d
```

---

## 🧪 Local Dry-Run Testing (Test directly on your PC)

You can test GitHub fetching and inspect the exact Telegram card layouts in your console without sending any messages or setting up Telegram credentials:

```bash
# Using CLI flag
cargo run -- --dry-run

# Or specifying repos via environment variable
DRY_RUN=true TRACKED_REPOS="rust-lang/rust,tokio-rs/tokio" cargo run
```

Sample Dry-Run Terminal Output:
```
==========================================================
🚀 DRY-RUN MODE ACTIVE: Running test poll cycle...
No messages will be sent to Telegram.
==========================================================

╔════════════════════════════════════════════════════════════════════════════╗
║  [DRY-RUN] UPDATE DETECTED: tokio-rs/tokio (Release)
╠════════════════════════════════════════════════════════════════════════════╣
🚀 <b>NEW RELEASE</b> • <b>tokio-rs/tokio</b>

📦 <b>Version:</b> <code>tokio-1.42.0</code>
📌 <b>Title:</b> tokio-1.42.0
🔗 <a href="https://github.com/tokio-rs/tokio/releases/tag/tokio-1.42.0"><b>Open Release Notes on GitHub</b></a>

<blockquote expandable>
This release adds support for...
</blockquote>
╚════════════════════════════════════════════════════════════════════════════╝
```

---

## 🤖 Interactive Telegram Commands

| Command | Syntax | Description |
|---|---|---|
| `/track` | `/track owner/repo` | Adds a new repository to watch list |
| `/untrack` | `/untrack owner/repo` | Removes a repository from watch list |
| `/list` | `/list` | Shows all tracked repositories & last seen versions |
| `/help` | `/help` | Shows help message and topic thread debug ID |

---

## ⚙️ Configuration Reference

| Option / Env Var | Config Key | Description |
|---|---|---|
| `TELEGRAM_BOT_TOKEN` | `telegram_bot_token` | Bot API token from @BotFather |
| `TELEGRAM_CHAT_ID` | `chat_id` | Supergroup Chat ID (negative integer) |
| `TELEGRAM_ARCHIVE_THREAD_ID` | `archive_thread_id` | Target topic thread ID |
| `GITHUB_TOKEN` | `github_token` | GitHub PAT (increases quota to 5000/hr) |
| `TRACKED_REPOS` | `repos` | Comma-separated repos (`owner/repo1,owner/repo2`) |
| `POLL_INTERVAL_SECS` | `poll_interval_secs` | Polling interval in seconds (default: 900) |
| `DB_PATH` | `db_path` | SQLite database file path |
| `DRY_RUN` / `--dry-run` | `dry_run` | Enable local dry-run test mode |

---

## 📄 License

MIT License.
