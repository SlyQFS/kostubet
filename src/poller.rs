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
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;
use teloxide::prelude::*;
use teloxide::types::InlineKeyboardMarkup;
use tracing::{error, info, warn};

/// Poller observability counters (shown in /debug).
pub static LAST_CYCLE_UNIX: AtomicI64 = AtomicI64::new(0);
pub static CYCLES: AtomicU64 = AtomicU64::new(0);
pub static CHECK_ERRORS: AtomicU64 = AtomicU64::new(0);

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tracing::instrument(skip(bot, db, config))]
pub async fn run_poller(bot: Bot, db: Database, config: Config) -> Result<()> {
    let github_client = GithubClient::new(config.github_token.clone())?;

    info!(
        "Запущен фоновый поллер GitHub (интервал: {} сек | Dry-run: {} | Pre-release: {})...",
        config.poll_interval_secs, config.dry_run, config.post_prereleases
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

                            // Break the whole cycle when the API quota is out:
                            // further requests would only burn errors.
                            let mut rate_limited_for = 0u64;

                            match github_client
                                .check_repo(
                                    &tool.owner,
                                    &tool.repo,
                                    tool.etag.as_deref(),
                                    tool.last_release.as_deref(),
                                    config.check_releases,
                                    config.check_commits,
                                    config.post_prereleases,
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
                                            let label = if a.variant == "release" {
                                                "⬇️ Скачать релиз".to_string()
                                            } else {
                                                format!("⬇️ Скачать ({})", a.variant)
                                            };
                                            (label, DownloadTarget::Url(a.url))
                                        })
                                        .collect();

                                    let post = PostData {
                                        title: format!(
                                            "{}/{} • {}",
                                            tool.owner, tool.repo, update.title
                                        ),
                                        description: tool.description.clone(),
                                        body: update.body,
                                        diff_url: Some(update.url),
                                        tags: tag_names,
                                        cover_image: None,
                                        download_buttons,
                                        suggested_by: tool.suggested_by.clone(),
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
                                                // Never wipe a stored ETag with None:
                                                // tag/commit updates carry no ETag,
                                                // and nulling the column would force
                                                // full re-fetches every cycle.
                                                let etag_to_store =
                                                    update.etag.as_deref().or(tool.etag.as_deref());
                                                if let Err(e) = db
                                                    .tools()
                                                    .update_last_release_and_etag(
                                                        tool.id,
                                                        Some(&update.id),
                                                        etag_to_store,
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
                                    let _ = db.tools().reset_tool_failures(tool.id).await;
                                }
                                Ok(CheckResult::NoUpdatesFound) => {
                                    info!(
                                        "Релизы/теги не найдены для {}/{}.",
                                        tool.owner, tool.repo
                                    );
                                    let _ = db.tools().reset_tool_failures(tool.id).await;
                                }
                                Ok(CheckResult::RepoNotFound) => {
                                    // Repository deleted or renamed: count
                                    // consecutive misses and alert admins once.
                                    let fails = db
                                        .tools()
                                        .bump_tool_failures(tool.id)
                                        .await
                                        .unwrap_or(0);
                                    warn!(
                                        "Репозиторий {}/{} не найден (404), серия: {}.",
                                        tool.owner, tool.repo, fails
                                    );
                                    if fails == 3 {
                                        let kb = InlineKeyboardMarkup::new(vec![vec![
                                            teloxide::types::InlineKeyboardButton::callback(
                                                "🗑 Убрать из отслеживания",
                                                format!("adm:repountrack:{}", tool.id),
                                            ),
                                        ]]);
                                        let _ = crate::handlers::callbacks::notify_admins(
                                            &bot,
                                            &db,
                                            format!(
                                                "⚠️ <b>Репозиторий {}/{}</b> не отвечает (404) три проверки подряд.\nВозможно, он удален или переименован.",
                                                tool.owner, tool.repo
                                            ),
                                            Some(kb),
                                            None,
                                        )
                                        .await;
                                    }
                                }
                                Ok(CheckResult::RateLimited { retry_after_secs }) => {
                                    rate_limited_for = retry_after_secs;
                                    warn!(
                                        "Лимит GitHub API исчерпан на {}/{}. Пауза до сброса квоты (~{} сек).",
                                        tool.owner, tool.repo, retry_after_secs
                                    );
                                }
                                Err(e) => {
                                    CHECK_ERRORS.fetch_add(1, Ordering::Relaxed);
                                    warn!(
                                        "Ошибка при проверке {}/{}: {:?}. Продолжение...",
                                        tool.owner, tool.repo, e
                                    );
                                }
                            }

                            if rate_limited_for > 0 {
                                // Stop polling the remaining repos this cycle
                                // and sleep until the quota resets.
                                tokio::time::sleep(Duration::from_secs(rate_limited_for)).await;
                                break;
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
        CYCLES.fetch_add(1, Ordering::Relaxed);
        LAST_CYCLE_UNIX.store(unix_now(), Ordering::Relaxed);
    }
}
