use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, IF_NONE_MATCH, USER_AGENT};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateType {
    Release,
    Tag,
    Commit,
}

impl std::fmt::Display for UpdateType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateType::Release => write!(f, "Release"),
            UpdateType::Tag => write!(f, "Tag"),
            UpdateType::Commit => write!(f, "Commit"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GithubUpdate {
    pub update_type: UpdateType,
    pub id: String, // release id, tag name, or commit sha
    pub tag_or_version: String,
    pub title: String,
    pub url: String,
    pub body: Option<String>,
    pub etag: Option<String>,
    pub sha: Option<String>,
}

#[derive(Debug, Clone)]
pub enum CheckResult {
    NotModified,
    NewUpdate(GithubUpdate),
    NoUpdatesFound,
}

pub struct GithubClient {
    client: Client,
    #[allow(dead_code)]
    token: Option<String>,
}

#[derive(Deserialize)]
struct GhRelease {
    id: u64,
    tag_name: String,
    name: Option<String>,
    html_url: String,
    body: Option<String>,
}

#[derive(Deserialize)]
struct GhCommitRef {
    sha: String,
}

#[derive(Deserialize)]
struct GhTag {
    name: String,
    commit: GhCommitRef,
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
            HeaderValue::from_static("kostubet-github-bot/0.1.0"),
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
            .build()
            .context("Failed to build HTTP client for GitHub API")?;

        Ok(Self { client, token })
    }

    fn check_rate_limit(&self, headers: &HeaderMap) {
        if let Some(rem_val) = headers.get("x-ratelimit-remaining") {
            if let Ok(rem_str) = rem_val.to_str() {
                if let Ok(rem) = rem_str.parse::<u32>() {
                    if rem < 10 {
                        warn!("GitHub API rate limit is very low! Remaining: {}", rem);
                    }
                }
            }
        }
    }

    pub async fn check_repo(
        &self,
        owner: &str,
        name: &str,
        cached_etag: Option<&str>,
        last_seen_id: Option<&str>,
        last_seen_sha: Option<&str>,
        check_releases: bool,
        check_tags: bool,
        check_commits: bool,
    ) -> Result<CheckResult> {
        // 1. Try Releases API (/repos/{owner}/{repo}/releases/latest)
        if check_releases {
            let release_url = format!("https://api.github.com/repos/{}/{}/releases/latest", owner, name);
            let mut req = self.client.get(&release_url);
            if let Some(etag) = cached_etag {
                req = req.header(IF_NONE_MATCH, etag);
            }

            let resp = req.send().await.with_context(|| format!("HTTP GET failed for {}", release_url))?;
            self.check_rate_limit(resp.headers());

            if resp.status() == StatusCode::NOT_MODIFIED {
                return Ok(CheckResult::NotModified);
            }

            let etag_header = resp
                .headers()
                .get("etag")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            if resp.status().is_success() {
                let release: GhRelease = resp.json().await.context("Failed to parse release JSON")?;
                let current_id = release.id.to_string();

                if last_seen_id == Some(&current_id) || last_seen_id == Some(&release.tag_name) {
                    return Ok(CheckResult::NotModified);
                }

                let title = release
                    .name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| release.tag_name.clone());

                return Ok(CheckResult::NewUpdate(GithubUpdate {
                    update_type: UpdateType::Release,
                    id: current_id,
                    tag_or_version: release.tag_name,
                    title,
                    url: release.html_url,
                    body: release.body,
                    etag: etag_header,
                    sha: None,
                }));
            } else if resp.status() != StatusCode::NOT_FOUND {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                warn!("GitHub releases returned status {}: {}", status, text);
            }
        }

        // 2. Fallback: Try Tags API (/repos/{owner}/{repo}/tags)
        if check_tags {
            let tags_url = format!("https://api.github.com/repos/{}/{}/tags?per_page=1", owner, name);
            let tags_resp = self
                .client
                .get(&tags_url)
                .send()
                .await
                .with_context(|| format!("HTTP GET failed for {}", tags_url))?;
            self.check_rate_limit(tags_resp.headers());

            if tags_resp.status().is_success() {
                let tags: Vec<GhTag> = tags_resp.json().await.context("Failed to parse tags JSON")?;
                if let Some(tag) = tags.into_iter().next() {
                    if last_seen_id == Some(&tag.name) || last_seen_sha.as_deref() == Some(&tag.commit.sha) {
                        return Ok(CheckResult::NotModified);
                    }

                    let tag_url = format!("https://github.com/{}/{}/releases/tag/{}", owner, name, tag.name);
                    return Ok(CheckResult::NewUpdate(GithubUpdate {
                        update_type: UpdateType::Tag,
                        id: tag.name.clone(),
                        tag_or_version: tag.name.clone(),
                        title: format!("Tag {}", tag.name),
                        url: tag_url,
                        body: None,
                        etag: None,
                        sha: Some(tag.commit.sha),
                    }));
                }
            }
        }

        // 3. Fallback: Try Commits API (/repos/{owner}/{repo}/commits)
        if check_commits {
            let commits_url = format!("https://api.github.com/repos/{}/{}/commits?per_page=1", owner, name);
            let commits_resp = self
                .client
                .get(&commits_url)
                .send()
                .await
                .with_context(|| format!("HTTP GET failed for {}", commits_url))?;
            self.check_rate_limit(commits_resp.headers());

            if commits_resp.status().is_success() {
                let commits: Vec<GhCommit> = commits_resp.json().await.context("Failed to parse commits JSON")?;
                if let Some(commit) = commits.into_iter().next() {
                    if last_seen_sha.as_deref() == Some(&commit.sha) || last_seen_id.as_deref() == Some(&commit.sha) {
                        return Ok(CheckResult::NotModified);
                    }

                    let short_sha = if commit.sha.len() >= 7 {
                        &commit.sha[..7]
                    } else {
                        &commit.sha
                    };

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
                        tag_or_version: short_sha.to_string(),
                        title: first_line,
                        url: commit.html_url,
                        body: Some(commit.commit.message),
                        etag: None,
                        sha: Some(commit.sha),
                    }));
                }
            }
        }

        Ok(CheckResult::NoUpdatesFound)
    }
}
