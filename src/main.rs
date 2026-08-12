mod config;
mod db;
mod formatter;
mod github;
mod telegram;

use anyhow::Result;
use config::Config;
use db::Database;
use formatter::format_update_message;
use github::{CheckResult, GithubClient};
use std::env;
use std::time::Duration;
use telegram::{handle_command, send_update, Command};
use teloxide::prelude::*;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

fn print_usage() {
    println!(
        "Kostubet GitHub Watcher Bot v0.1.0\n\n\
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
          ADMIN_USER_IDS             Список Telegram User ID администраторов через запятую\n  \
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

    // Initialize tracing / logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("kostubet_github=info,teloxide=info")),
        )
        .init();

    info!("Запуск Kostubet GitHub Watcher Bot v0.1.0...");

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

    // Initialize database
    let db = Database::new(&config.db_path).await?;
    info!("База данных инициализирована: {}", config.db_path);

    // Sync static repos from config file/env to database
    for repo in &config.repos {
        match db.add_repo(&repo.owner, &repo.name).await {
            Ok(true) => info!("Зарегистрирован репозиторий в базе: {}", repo.full_name()),
            Ok(false) => {} // Already tracked
            Err(e) => warn!("Не удалось добавить репозиторий {} в базу: {:?}", repo.full_name(), e),
        }
    }

    // Initialize GitHub client
    let github_client = GithubClient::new(config.github_token.clone())?;

    // Start Telegram command listener REPL
    if !config.telegram_bot_token.is_empty() {
        let bot = Bot::new(&config.telegram_bot_token);
        let bot_clone = bot.clone();
        let db_clone = db.clone();
        let target_chat_id = config.chat_id;
        let target_thread_id = config.archive_thread_id;
        let admin_ids = config.admin_user_ids.clone();

        tokio::spawn(async move {
            info!("Запущен слушатель команд Telegram (в ЛС)...");
            Command::repl(bot_clone, move |b, msg, cmd| {
                let db_inner = db_clone.clone();
                let admin_ids = admin_ids.clone();
                async move {
                    handle_command(b, msg, cmd, db_inner, target_chat_id, target_thread_id, admin_ids).await
                }
            })
            .await;
        });
    }

    info!(
        "Интервал проверки GitHub: {} сек | Тестовый режим: {}",
        config.poll_interval_secs, config.dry_run
    );

    if config.chat_id == 0 {
        warn!("⚠️ TELEGRAM_CHAT_ID не задан (0)! Бот работает в режиме автообнаружения. Напишите /help в ЛС боту!");
    }

    let bot = Bot::new(&config.telegram_bot_token);

    loop {
        if config.chat_id != 0 {
            match db.get_tracked_repos().await {
                Ok(repos) => {
                    if repos.is_empty() {
                        warn!("Список репозиториев пуст! Добавьте их через TRACKED_REPOS в .env или команду /track в ЛС.");
                    } else {
                        info!("Проверка {} отслеживаемых репозиториев...", repos.len());
                        for repo in &repos {
                            info!("Проверка репозитория {}/{}...", repo.owner, repo.name);
                            match github_client
                                .check_repo(
                                    &repo.owner,
                                    &repo.name,
                                    repo.etag.as_deref(),
                                    repo.last_seen_id.as_deref(),
                                    repo.last_seen_sha.as_deref(),
                                    config.check_releases,
                                    config.check_tags,
                                    config.check_commits,
                                )
                                .await
                            {
                                Ok(CheckResult::NewUpdate(update)) => {
                                    info!(
                                        "Обнаружено новое обновление ({}) для {}/{}: {}",
                                        update.update_type, repo.owner, repo.name, update.title
                                    );

                                    let html_msg = format_update_message(
                                        &repo.owner,
                                        &repo.name,
                                        &update,
                                        config.dry_run,
                                    );

                                    match send_update(
                                        &bot,
                                        config.chat_id,
                                        config.archive_thread_id,
                                        &html_msg,
                                    )
                                    .await
                                    {
                                        Ok(_) => {
                                            info!(
                                                "Карточка успешно отправлена в Telegram для {}/{}",
                                                repo.owner, repo.name
                                            );

                                            if config.dry_run {
                                                info!(
                                                    "[DRY-RUN] Пропуск обновления записи в БД для {}/{}.",
                                                    repo.owner, repo.name
                                                );
                                            } else {
                                                if let Err(e) = db
                                                    .mark_seen(
                                                        &repo.owner,
                                                        &repo.name,
                                                        Some(&update.id),
                                                        update.sha.as_deref(),
                                                        update.etag.as_deref(),
                                                    )
                                                    .await
                                                {
                                                    error!(
                                                        "Не удалось обновить запись в БД для {}/{}: {:?}",
                                                        repo.owner, repo.name, e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!(
                                                "Ошибка отправки карточки в Telegram для {}/{}: {:?}. Повтор в следующем цикле.",
                                                repo.owner, repo.name, e
                                            );
                                        }
                                    }
                                }
                                Ok(CheckResult::NotModified) => {
                                    info!("Репозиторий {}/{} без изменений (304 / ETag совпадает).", repo.owner, repo.name);
                                }
                                Ok(CheckResult::NoUpdatesFound) => {
                                    info!("Релизы/теги/коммиты не найдены для {}/{}.", repo.owner, repo.name);
                                }
                                Err(e) => {
                                    warn!("Ошибка при проверке репозитория {}/{}: {:?}. Продолжение...", repo.owner, repo.name, e);
                                }
                            }

                            // Stagger requests to avoid bursting APIs
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
                Err(e) => {
                    error!("Ошибка базы данных при получении списка репозиториев: {:?}", e);
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    }
}
