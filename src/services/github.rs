//! GitHub API client with conditional (ETag) requests for release/tag/commit polling.

use crate::services::apk_variant::detect_variant;
use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, IF_NONE_MATCH, USER_AGENT};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::OnceLock;
use std::time::Duration;
use tracing::warn;

const API_BASE: &str = "https://api.github.com";
/// Hard HTTP timeout: a hung connection must never stall the poller loop.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// How many newest releases to inspect per poll (catches bursts between polls).
const RELEASES_PAGE_SIZE: usize = 10;

/// Last observed GitHub API quota (for /debug diagnostics).
pub static RATE_REMAINING: AtomicU32 = AtomicU32::new(u32::MAX);
pub static RATE_RESET_UNIX: AtomicI64 = AtomicI64::new(0);

static GLOBAL_CLIENT: OnceLock<GithubClient> = OnceLock::new();

/// Initializes the process-wide GitHub client (called once at startup).
pub fn init_global(token: Option<String>) -> Result<()> {
    let client = GithubClient::new(token)?;
    let _ = GLOBAL_CLIENT.set(client);
    Ok(())
}

/// Process-wide GitHub client (handlers use it for one-off validation calls).
pub fn global() -> Option<&'static GithubClient> {
    GLOBAL_CLIENT.get()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateType {
    Release,
    Commit,
}

impl std::fmt::Display for UpdateType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateType::Release => write!(f, "Release"),
            UpdateType::Commit => write!(f, "Commit"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApkAsset {
    pub variant: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct GithubUpdate {
    pub update_type: UpdateType,
    /// Release id, tag name, or commit sha — persisted as `last_release`.
    pub id: String,
    pub title: String,
    pub url: String,
    pub body: Option<String>,
    /// ETag of the releases response (only the releases endpoint is
    /// conditionally polled; commits dedup by sha).
    pub etag: Option<String>,
    pub apk_assets: Vec<ApkAsset>,
}

#[derive(Debug, Clone)]
pub enum CheckResult {
    NotModified,
    NewUpdate(GithubUpdate),
    NoUpdatesFound,
    /// The repository no longer exists (404 from every checked endpoint).
    RepoNotFound,
    /// GitHub API quota exhausted; `retry_after_secs` until the reset.
    RateLimited { retry_after_secs: u64 },
}

pub struct GithubClient {
    client: Client,
}

#[derive(Deserialize, Debug, Clone)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Deserialize, Debug, Clone)]
struct GhRelease {
    id: u64,
    tag_name: String,
    name: Option<String>,
    html_url: String,
    body: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhCommitDetail {
    message: String,
}

#[derive(Deserialize)]
struct GhCommit {
    sha: String,
    html_url: String,
    commit: GhCommitDetail,
}

impl GithubClient {
    pub fn new(token: Option<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("kostubet-github-bot/0.2.0"),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );

        if let Some(ref t) = token {
            let mut auth_val = HeaderValue::from_str(&format!("Bearer {}", t))
                .context("Invalid GitHub token header value")?;
            auth_val.set_sensitive(true);
            headers.insert(AUTHORIZATION, auth_val);
        }

        let client = Client::builder()
            .default_headers(headers)
            .timeout(HTTP_TIMEOUT)
            .build()
            .context("Failed to build HTTP client for GitHub API")?;

        Ok(Self { client })
    }

    /// Detects an exhausted rate limit from response headers/status and
    /// returns the seconds to wait until the quota resets.
    fn rate_limited_for(&self, status: StatusCode, headers: &HeaderMap) -> Option<u64> {
        let remaining = headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u32>().ok());

        // Track the observed quota for diagnostics (/debug).
        if let Some(rem) = remaining {
            RATE_REMAINING.store(rem, Ordering::Relaxed);
            if let Some(reset) = headers
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<i64>().ok())
            {
                RATE_RESET_UNIX.store(reset, Ordering::Relaxed);
            }
        }

        let exhausted = remaining == Some(0)
            || (status == StatusCode::FORBIDDEN && remaining.is_some_and(|r| r < 5));
        if !exhausted {
            if let Some(rem) = remaining {
                if rem < 10 {
                    warn!("GitHub API rate limit is very low! Remaining: {}", rem);
                }
            }
            return None;
        }

        let reset = headers
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Some((reset - now).clamp(60, 3600) as u64)
    }

    /// One-off existence check used when adding repositories
    /// (catches typos before they become dead trackers).
    pub async fn repo_exists(&self, owner: &str, name: &str) -> Result<bool> {
        let url = format!("{}/repos/{}/{}", API_BASE, owner, name);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("HTTP GET failed for {}", url))?;

        match resp.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            s => anyhow::bail!("GitHub returned {} for {}", s, url),
        }
    }

    /// Brief info about the current newest release (id + etag), used for
    /// "silent" tracking setup: the poller skips the already-existing release.
    pub async fn latest_release_brief(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<(String, Option<String>)>> {
        let url = format!(
            "{}/repos/{}/{}/releases?per_page=1",
            API_BASE, owner, name
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("HTTP GET failed for {}", url))?;

        if resp.status() != StatusCode::OK {
            return Ok(None);
        }

        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let releases: Vec<GhRelease> = resp.json().await.context("Failed to parse releases JSON")?;
        Ok(releases
            .into_iter()
            .next()
            .map(|r| (r.id.to_string(), etag)))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn check_repo(
        &self,
        owner: &str,
        name: &str,
        cached_etag: Option<&str>,
        last_seen_id: Option<&str>,
        check_releases: bool,
        check_commits: bool,
        include_prereleases: bool,
    ) -> Result<CheckResult> {
        // Every endpoint that answered 404 counts; if ALL checked endpoints
        // are gone, the repository itself no longer exists.
        let mut endpoints_checked = 0usize;
        let mut endpoints_404 = 0usize;

        // 1. Releases & Pre-releases (including nightly builds): poll the LIST endpoint (per_page=N)
        // instead of /releases/latest — the latter silently skips pre-releases, so
        // repositories publishing beta / nightly builds would never produce updates.
        if check_releases {
            endpoints_checked += 1;
            let release_url = format!(
                "{}/repos/{}/{}/releases?per_page={}",
                API_BASE, owner, name, RELEASES_PAGE_SIZE
            );
            let mut req = self.client.get(&release_url);
            if let Some(etag) = cached_etag {
                req = req.header(IF_NONE_MATCH, etag);
            }

            let resp = req
                .send()
                .await
                .with_context(|| format!("HTTP GET failed for {}", release_url))?;

            if let Some(retry_after) = self.rate_limited_for(resp.status(), resp.headers()) {
                return Ok(CheckResult::RateLimited {
                    retry_after_secs: retry_after,
                });
            }

            if resp.status() == StatusCode::NOT_MODIFIED {
                return Ok(CheckResult::NotModified);
            }

            let etag_header = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            if resp.status() == StatusCode::NOT_FOUND {
                endpoints_404 += 1;
                warn!("No releases found for {}/{} (404)", owner, name);
            } else if resp.status().is_success() {
                let mut releases: Vec<GhRelease> = resp
                    .json()
                    .await
                    .context("Failed to parse releases JSON")?;

                // Optional pre-release filter (repos flooding with betas).
                if !include_prereleases {
                    releases.retain(|r| !r.prerelease);
                }

                // The list is sorted newest-first. If the newest entry is the
                // already-seen one — nothing changed. If a seen release sits
                // deeper in the list, several releases landed between polls:
                // post only the newest instead of flooding the chat.
                if let Some(newest) = releases.first() {
                    let seen_at = releases.iter().position(|r| {
                        last_seen_id == Some(r.id.to_string().as_str())
                            || last_seen_id == Some(r.tag_name.as_str())
                    });

                    if seen_at == Some(0) {
                        return Ok(CheckResult::NotModified);
                    }

                    let title = newest
                        .name
                        .clone()
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or_else(|| newest.tag_name.clone());
                    let title = if newest.prerelease {
                        format!("{} [pre-release]", title)
                    } else {
                        title
                    };

                    // Filter and organize binary/package assets (.apk, .zip, .7z)
                    let all_apk_assets: Vec<GhAsset> = newest
                        .assets
                        .iter()
                        .filter(|a| {
                            let name = a.name.to_lowercase();
                            name.ends_with(".apk") || name.ends_with(".zip") || name.ends_with(".7z")
                        })
                        .cloned()
                        .collect();

                    let has_archives = all_apk_assets
                        .iter()
                        .any(|a| {
                            let name = a.name.to_lowercase();
                            name.ends_with(".zip") || name.ends_with(".7z")
                        });

                    let apk_assets: Vec<ApkAsset> = if has_archives {
                        vec![ApkAsset {
                            variant: "release".to_string(),
                            url: newest.html_url.clone(),
                        }]
                    } else {
                        let apk_files: Vec<&GhAsset> = all_apk_assets
                            .iter()
                            .filter(|a| a.name.to_lowercase().ends_with(".apk"))
                            .collect();

                        if apk_files.is_empty() {
                            Vec::new()
                        } else if let Some(universal) = apk_files
                            .iter()
                            .find(|a| detect_variant(&a.name) == Some("universal") || a.name.to_lowercase().contains("universal"))
                        {
                            vec![ApkAsset {
                                variant: "universal".to_string(),
                                url: universal.browser_download_url.clone(),
                            }]
                        } else if let Some(v8) = apk_files
                            .iter()
                            .find(|a| detect_variant(&a.name) == Some("arm64-v8a"))
                        {
                            vec![ApkAsset {
                                variant: "arm64-v8a".to_string(),
                                url: v8.browser_download_url.clone(),
                            }]
                        } else if apk_files.len() == 1 && detect_variant(&apk_files[0].name) != Some("armeabi-v7a") {
                            let single = apk_files[0];
                            vec![ApkAsset {
                                variant: detect_variant(&single.name).unwrap_or("apk").to_string(),
                                url: single.browser_download_url.clone(),
                            }]
                        } else {
                            vec![ApkAsset {
                                variant: "release".to_string(),
                                url: newest.html_url.clone(),
                            }]
                        }
                    };

                    return Ok(CheckResult::NewUpdate(GithubUpdate {
                        update_type: UpdateType::Release,
                        id: newest.id.to_string(),
                        title,
                        url: newest.html_url.clone(),
                        body: newest.body.clone(),
                        etag: etag_header,
                        apk_assets,
                    }));
                }
            } else {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                warn!("GitHub releases returned status {}: {}", status, text);
                return Ok(CheckResult::NoUpdatesFound);
            }
        }

        // 2. Fallback: Commits API (/repos/{owner}/{repo}/commits)
        if check_commits {
            endpoints_checked += 1;
            let commits_url = format!("{}/repos/{}/{}/commits?per_page=1", API_BASE, owner, name);
            let commits_resp = self
                .client
                .get(&commits_url)
                .send()
                .await
                .with_context(|| format!("HTTP GET failed for {}", commits_url))?;

            if let Some(retry_after) =
                self.rate_limited_for(commits_resp.status(), commits_resp.headers())
            {
                return Ok(CheckResult::RateLimited {
                    retry_after_secs: retry_after,
                });
            }

            if commits_resp.status() == StatusCode::NOT_FOUND {
                endpoints_404 += 1;
            } else if commits_resp.status().is_success() {
                let commits: Vec<GhCommit> = commits_resp
                    .json()
                    .await
                    .context("Failed to parse commits JSON")?;
                if let Some(commit) = commits.into_iter().next() {
                    if last_seen_id == Some(&commit.sha) {
                        return Ok(CheckResult::NotModified);
                    }

                    let first_line = commit
                        .commit
                        .message
                        .lines()
                        .next()
                        .unwrap_or("New commit")
                        .to_string();

                    return Ok(CheckResult::NewUpdate(GithubUpdate {
                        update_type: UpdateType::Commit,
                        id: commit.sha.clone(),
                        title: first_line,
                        url: commit.html_url,
                        body: Some(commit.commit.message),
                        etag: None,
                        apk_assets: Vec::new(),
                    }));
                }
            }
        }

        // All checked endpoints 404 -> repository deleted/renamed.
        if endpoints_checked > 0 && endpoints_404 == endpoints_checked {
            return Ok(CheckResult::RepoNotFound);
        }

        Ok(CheckResult::NoUpdatesFound)
    }
}
