//! Kostubet GitHub & APK Watcher Bot entrypoint.
//!
//! Initializes logging, loads configuration, opens SQLite storage with WAL mode,
//! registers bot command autocomplete with Telegram Bot API, spawns background GitHub poller,
//! and runs the teloxide message/callback dispatcher.

mod config;
mod db;
mod dialogue;
mod handlers;
mod poller;
mod services;
mod strings;

use anyhow::Result;
use config::Config;
use db::Database;
use dialogue::{
    handle_admin_message, handle_edit_message, handle_submit_message, handle_suggest_message,
    BotDialogue, DialogueState, SqliteDialogueStorage,
};
use handlers::callbacks::handle_callback;
use handlers::{handle_command, Command};
use poller::run_poller;
use std::env;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

fn print_usage() {
    println!(
        "Kostubet GitHub & APK Watcher Bot v0.2.0\n\n\
        ИСПОЛЬЗОВАНИЕ:\n  \
          kostubet-github [ПАРАМЕТРЫ]\n\n\
        ПАРАМЕТРЫ:\n  \
          -d, --dry-run    Запуск в тестовом режиме (отправка карточек в Telegram без обновления базы данных).\n  \
          -h, --help       Показать эту справку и выйти.\n\n\
        ПЕРЕМЕННЫЕ ОКРУЖЕНИЯ:\n  \
          TELEGRAM_BOT_TOKEN         Токен бота Telegram от @BotFather\n  \
          TELEGRAM_CHAT_ID           ID группы (например, -1001234567890)\n  \
          TELEGRAM_ARCHIVE_THREAD_ID ID топика форума (message_thread_id)\n  \
          GITHUB_TOKEN               Персональный токен GitHub (лимит 5000 запр/час)\n  \
          TRACKED_REPOS              Список репозиториев через запятую ('owner/repo1,owner/repo2')\n  \
          ADMIN_USER_IDS             Список Telegram User ID администраторов через запятую (первый — владелец)\n  \
          POLL_INTERVAL_SECS         Интервал проверки в секундах (по умолчанию: 900 = 15 мин)\n  \
          DRY_RUN                    Установите '1' или 'true' для тестового режима\n"
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let mut cli_dry_run = false;

    for arg in &args[1..] {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            "-d" | "--dry-run" => {
                cli_dry_run = true;
            }
            _ => {}
        }
    }

    // Load .env for local runs (docker/systemd inject env vars directly).
    let _ = dotenvy::dotenv();

    // Initialize tracing / logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("kostubet_github=info,teloxide=info")),
        )
        .init();

    info!("Запуск Kostubet GitHub Watcher Bot v0.2.0...");

    // Load configuration
    let config = Config::load(None::<&str>, cli_dry_run)?;

    // Validate configuration requirements
    if let Err(e) = config.validate() {
        error!("Ошибка конфигурации: {}", e);
        anyhow::bail!(e);
    }

    if config.dry_run {
        info!("🧪 ТЕСТОВЫЙ РЕЖИМ (DRY-RUN) АКТИВЕН: Карточки отправляются в Telegram, но БД не обновляется.");
    }

    // Announce the effective publication destination so a misconfigured
    // topic is visible immediately in the logs.
    match config.chat_id {
        0 => {}
        cid => match config.archive_thread_id {
            Some(tid) => info!(
                "Публикация карточек: чат {} в топик {} (проверьте, что бот состоит в группе и тема существует)",
                cid, tid
            ),
            None => info!(
                "Публикация карточек: чат {} без указания топика (TELEGRAM_ARCHIVE_THREAD_ID не задан — карточки пойдут в «Общий»)",
                cid
            ),
        },
    }

    // Initialize database
    let db = Database::new(&config.db_path).await?;
    info!("База данных инициализирована: {}", config.db_path);

    // Seed admins from config if database is empty
    db.admins().seed_admins(&config.admin_user_ids).await?;

    // Sync static repos from config file/env to database
    for repo in &config.repos {
        let _ = db.tools().add_tool(&repo.owner, &repo.name, 0, None, None).await;
    }

    let bot = Bot::new(&config.telegram_bot_token);
    let storage = SqliteDialogueStorage::new(db.pool().clone());

    // Process-wide GitHub client for one-off validation calls
    // (repo existence checks on /track, suggestion approval, admin dialogues).
    if let Err(e) = crate::services::github::init_global(config.github_token.clone()) {
        error!("Не удалось инициализировать глобальный GitHub-клиент: {:?}", e);
    }

    // Register bot commands with Telegram UI autocomplete
    if let Err(e) = bot.set_my_commands(Command::bot_commands()).await {
        error!(
            "Не удалось зарегистрировать подсказки команд Telegram: {:?}",
            e
        );
    } else {
        info!("Подсказки команд Telegram успешно зарегистрированы.");
    }

    let target_chat_id = config.chat_id;
    let target_thread_id = config.archive_thread_id;

    // Spawn background poller loop
    let poller_bot = bot.clone();
    let poller_db = db.clone();
    let poller_config = config.clone();
    tokio::spawn(async move {
        if let Err(e) = run_poller(poller_bot, poller_db, poller_config).await {
            error!("Ошибка в работе фонового поллера: {:?}", e);
        }
    });

    // Build dptree handler schema
    let message_tree = Update::filter_message()
        .enter_dialogue::<Message, SqliteDialogueStorage, DialogueState>()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(
            dptree::filter(|state: DialogueState| matches!(state, DialogueState::SubmitApk(_)))
                .endpoint(
                    |bot: Bot,
                     msg: Message,
                     dialogue: BotDialogue,
                     state: DialogueState,
                     db: Database| async move {
                        if let DialogueState::SubmitApk(s) = state {
                            let _ = handle_submit_message(bot, msg, dialogue, s, db).await;
                        }
                        Ok::<(), teloxide::RequestError>(())
                    },
                ),
        )
        .branch(
            dptree::filter(|state: DialogueState| matches!(state, DialogueState::EditApk(_)))
                .endpoint(
                    |bot: Bot,
                     msg: Message,
                     dialogue: BotDialogue,
                     state: DialogueState,
                     db: Database| async move {
                        if let DialogueState::EditApk(s) = state {
                            let _ = handle_edit_message(bot, msg, dialogue, s, db).await;
                        }
                        Ok::<(), teloxide::RequestError>(())
                    },
                ),
        )
        .branch(
            dptree::filter(|state: DialogueState| matches!(state, DialogueState::Admin(_)))
                .endpoint(
                    |bot: Bot,
                     msg: Message,
                     dialogue: BotDialogue,
                     state: DialogueState,
                     db: Database| async move {
                        if let DialogueState::Admin(s) = state {
                            let _ = handle_admin_message(bot, msg, dialogue, *s, db).await;
                        }
                        Ok::<(), teloxide::RequestError>(())
                    },
                ),
        )
        .branch(
            dptree::filter(|state: DialogueState| matches!(state, DialogueState::Suggest(_)))
                .endpoint(
                    |bot: Bot,
                     msg: Message,
                     dialogue: BotDialogue,
                     state: DialogueState,
                     db: Database| async move {
                        if let DialogueState::Suggest(s) = state {
                            let _ = handle_suggest_message(bot, msg, dialogue, s, db).await;
                        }
                        Ok::<(), teloxide::RequestError>(())
                    },
                ),
        );

    let callback_tree = Update::filter_callback_query()
        .enter_dialogue::<CallbackQuery, SqliteDialogueStorage, DialogueState>()
        .endpoint(
            |bot: Bot,
             q: CallbackQuery,
             dialogue: BotDialogue,
             db: Database,
             target_chat_id: i64,
             target_thread_id: Option<i64>| async move {
                let _ =
                    handle_callback(bot, q, dialogue, db, target_chat_id, target_thread_id).await;
                Ok::<(), teloxide::RequestError>(())
            },
        );

    let schema = dptree::entry().branch(message_tree).branch(callback_tree);

    info!("Запуск диспетчера сообщений и команд Telegram...");
    Dispatcher::builder(bot, schema)
        .dependencies(dptree::deps![storage, db, target_chat_id, target_thread_id])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}
