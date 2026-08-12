use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct RepoConfig {
    pub owner: String,
    pub name: String,
}

impl RepoConfig {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() == 2 && !parts[0].trim().is_empty() && !parts[1].trim().is_empty() {
            Some(Self {
                owner: parts[0].trim().to_string(),
                name: parts[1].trim().to_string(),
            })
        } else {
            None
        }
    }
}

pub fn parse_repo_list(s: &str) -> Vec<RepoConfig> {
    s.split(',')
        .filter_map(|item| RepoConfig::parse(item.trim()))
        .collect()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub telegram_bot_token: String,
    pub chat_id: i64,
    pub archive_thread_id: Option<i64>,
    pub github_token: Option<String>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
    #[serde(default)]
    pub admin_user_ids: Vec<i64>,
    #[serde(default = "default_true")]
    pub check_releases: bool,
    #[serde(default = "default_true")]
    pub check_tags: bool,
    #[serde(default = "default_true")]
    pub check_commits: bool,
    #[serde(default)]
    pub dry_run: bool,
}

fn default_true() -> bool {
    true
}

fn default_poll_interval() -> u64 {
    900 // 15 minutes
}

fn default_db_path() -> String {
    "kostubet.db".to_string()
}

impl Config {
    pub fn load<P: AsRef<Path>>(path: Option<P>, cli_dry_run: bool) -> Result<Self> {
        let config_file_path = path
            .map(|p| p.as_ref().to_path_buf())
            .or_else(|| {
                if Path::new("config.toml").exists() {
                    Some(Path::new("config.toml").to_path_buf())
                } else {
                    None
                }
            });

        let mut config: Config = if let Some(ref p) = config_file_path {
            let content = fs::read_to_string(p)
                .with_context(|| format!("Failed to read config file at {}", p.display()))?;
            toml::from_str(&content)
                .with_context(|| format!("Failed to parse TOML config file at {}", p.display()))?
        } else {
            Config {
                telegram_bot_token: String::new(),
                chat_id: 0,
                archive_thread_id: None,
                github_token: None,
                poll_interval_secs: default_poll_interval(),
                db_path: default_db_path(),
                repos: Vec::new(),
                admin_user_ids: Vec::new(),
                check_releases: true,
                check_tags: true,
                check_commits: true,
                dry_run: false,
            }
        };

        if cli_dry_run {
            config.dry_run = true;
        }

        if let Ok(dry_env) = env::var("DRY_RUN") {
            let val = dry_env.to_lowercase();
            if val == "1" || val == "true" || val == "yes" {
                config.dry_run = true;
            }
        }

        if let Ok(token) = env::var("TELEGRAM_BOT_TOKEN") {
            if !token.trim().is_empty() {
                config.telegram_bot_token = token.trim().to_string();
            }
        }

        if let Ok(chat_id_str) = env::var("TELEGRAM_CHAT_ID").or_else(|_| env::var("CHAT_ID")) {
            if let Ok(cid) = chat_id_str.trim().parse::<i64>() {
                config.chat_id = cid;
            }
        }

        if let Ok(thread_id_str) = env::var("TELEGRAM_ARCHIVE_THREAD_ID").or_else(|_| env::var("ARCHIVE_THREAD_ID")) {
            if let Ok(tid) = thread_id_str.trim().parse::<i64>() {
                config.archive_thread_id = Some(tid);
            }
        }

        if let Ok(gh_token) = env::var("GITHUB_TOKEN") {
            if !gh_token.trim().is_empty() {
                config.github_token = Some(gh_token.trim().to_string());
            }
        }

        if let Ok(poll_str) = env::var("POLL_INTERVAL_SECS") {
            if let Ok(interval) = poll_str.trim().parse::<u64>() {
                config.poll_interval_secs = interval;
            }
        }

        if let Ok(db_path) = env::var("DB_PATH").or_else(|_| env::var("DATABASE_URL")) {
            if !db_path.trim().is_empty() {
                let clean_path = db_path.trim().trim_start_matches("sqlite://");
                config.db_path = clean_path.to_string();
            }
        }

        if let Ok(admin_env) = env::var("ADMIN_USER_IDS").or_else(|_| env::var("ADMIN_IDS")) {
            for id_str in admin_env.split(',') {
                if let Ok(id) = id_str.trim().parse::<i64>() {
                    if !config.admin_user_ids.contains(&id) {
                        config.admin_user_ids.push(id);
                    }
                }
            }
        }

        if let Ok(val) = env::var("CHECK_RELEASES") {
            let v = val.to_lowercase();
            config.check_releases = v == "1" || v == "true" || v == "yes";
        }

        if let Ok(val) = env::var("CHECK_TAGS") {
            let v = val.to_lowercase();
            config.check_tags = v == "1" || v == "true" || v == "yes";
        }

        if let Ok(val) = env::var("CHECK_COMMITS") {
            let v = val.to_lowercase();
            config.check_commits = v == "1" || v == "true" || v == "yes";
        }

        if let Ok(tracked_env) = env::var("TRACKED_REPOS") {
            let env_repos = parse_repo_list(&tracked_env);
            for r in env_repos {
                if !config.repos.contains(&r) {
                    config.repos.push(r);
                }
            }
        }

        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.telegram_bot_token.trim().is_empty() {
            anyhow::bail!("telegram_bot_token is missing! Set it in config.toml, .env, or via TELEGRAM_BOT_TOKEN env var.");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_config_parse() {
        let parsed = RepoConfig::parse("tokio-rs/tokio").unwrap();
        assert_eq!(parsed.owner, "tokio-rs");
        assert_eq!(parsed.name, "tokio");
        assert_eq!(parsed.full_name(), "tokio-rs/tokio");

        assert!(RepoConfig::parse("invalid").is_none());
        assert!(RepoConfig::parse("/repo").is_none());
        assert!(RepoConfig::parse("owner/").is_none());
    }

    #[test]
    fn test_parse_repo_list() {
        let repos = parse_repo_list("rust-lang/rust, tokio-rs/tokio, invalid_entry");
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].full_name(), "rust-lang/rust");
        assert_eq!(repos[1].full_name(), "tokio-rs/tokio");
    }
}
