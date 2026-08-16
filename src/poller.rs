//! Background polling service for tracked GitHub repositories.
//!
//! Periodically queries GitHub API for new releases, tags, or commits,
//! utilizing conditional HTTP ETags (`If-None-Match`) to conserve API quotas,
//! and dispatches formatted update posts to Telegram.

use crate::config::Config;
use crate::db::tags::ItemType;
use crate::db::Database;
use crate::services::github::{CheckResult, GithubClient};
use crate::services::render::{send_post, DownloadTarget, PostData};
use anyhow::Result;
use std::time::Duration;
use teloxide::prelude::*;
use tracing::{error, info, warn};

#[tracing::instrument(skip(bot, db, config))]
pub async fn run_poller(bot: Bot, db: Database, config: Config) -> Result<()> {
    let github_client = GithubClient::new(config.github_token.clone())?;

    info!(
        "Запущен фоновый поллер GitHub (интервал: {} сек | Dry-run: {})...",
        config.poll_interval_secs, config.dry_run
    );

    loop {
        if config.chat_id != 0 {
            match db.tools().list_tools().await {
                Ok(tools) => {
                    if tools.is_empty() {
                        warn!("Список отслеживаемых инструментов пуст! Добавьте их через команду /track в ЛС.");
                    } else {
                        info!("Проверка {} отслеживаемых инструментов...", tools.len());
                        for tool in &tools {
                            info!("Проверка инструмента {}/{}...", tool.owner, tool.repo);

                            match github_client
                                .check_repo(
                                    &tool.owner,
                                    &tool.repo,
                                    tool.etag.as_deref(),
                                    tool.last_release.as_deref(),
                                    None,
                                    config.check_releases,
                                    config.check_tags,
                                    config.check_commits,
                                )
                                .await
                            {
                                Ok(CheckResult::NewUpdate(update)) => {
                                    info!(
                                        "Обнаружено новое обновление ({}) для {}/{}: {}",
                                        update.update_type, tool.owner, tool.repo, update.title
                                    );

                                    let tags = db
                                        .tags()
                                        .get_tags_for_item(ItemType::Tool, tool.id)
                                        .await
                                        .unwrap_or_default();

                                    let tag_names: Vec<String> =
                                        tags.into_iter().map(|t| t.name).collect();

                                    let download_buttons: Vec<(String, DownloadTarget)> = update
                                        .apk_assets
                                        .into_iter()
                                        .map(|a| {
                                            (
                                                format!("⬇️ Скачать ({})", a.variant),
                                                DownloadTarget::Url(a.url),
                                            )
                                        })
                                        .collect();

                                    let post = PostData {
                                        title: format!(
                                            "{}/{} • {}",
                                            tool.owner, tool.repo, update.title
                                        ),
                                        body: update.body,
                                        diff_url: Some(update.url),
                                        tags: tag_names,
                                        cover_image: None,
                                        download_buttons,
                                    };

                                    match send_post(
                                        &bot,
                                        config.chat_id,
                                        config.archive_thread_id,
                                        &post,
                                    )
                                    .await
                                    {
                                        Ok(_) => {
                                            info!(
                                                "Карточка успешно отправлена в Telegram для {}/{}",
                                                tool.owner, tool.repo
                                            );

                                            if config.dry_run {
                                                info!(
                                                    "[DRY-RUN] Пропуск обновления записи в БД для {}/{}.",
                                                    tool.owner, tool.repo
                                                );
                                            } else {
                                                if let Err(e) = db
                                                    .tools()
                                                    .update_last_release_and_etag(
                                                        tool.id,
                                                        Some(&update.id),
                                                        update.etag.as_deref(),
                                                    )
                                                    .await
                                                {
                                                    error!(
                                                        "Не удалось обновить last_release и etag в БД для {}/{}: {:?}",
                                                        tool.owner, tool.repo, e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            error!(
                                                "Ошибка отправки карточки в Telegram для {}/{}: {:?}. Повтор в следующем цикле.",
                                                tool.owner, tool.repo, e
                                            );
                                        }
                                    }
                                }
                                Ok(CheckResult::NotModified) => {
                                    info!(
                                        "Инструмент {}/{} без изменений (304 / ETag match).",
                                        tool.owner, tool.repo
                                    );
                                }
                                Ok(CheckResult::NoUpdatesFound) => {
                                    info!(
                                        "Релизы/теги не найдены для {}/{}.",
                                        tool.owner, tool.repo
                                    );
                                }
                                Err(e) => {
                                    warn!(
                                        "Ошибка при проверке {}/{}: {:?}. Продолжение...",
                                        tool.owner, tool.repo, e
                                    );
                                }
                            }

                            // Stagger requests to avoid bursting APIs
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
                Err(e) => {
                    error!(
                        "Ошибка базы данных при получении списка инструментов: {:?}",
                        e
                    );
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs)).await;
    }
}
