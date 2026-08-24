//! Telegraph API client and Instant View guide publisher.
//!
//! Provides automated article creation on `telegra.ph` with Instant View support
//! from simple text guides, structured notes, and external links.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static TELEGRAPH_ACCESS_TOKEN: OnceLock<tokio::sync::RwLock<Option<String>>> = OnceLock::new();

fn get_token_lock() -> &'static tokio::sync::RwLock<Option<String>> {
    TELEGRAPH_ACCESS_TOKEN.get_or_init(|| tokio::sync::RwLock::new(None))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TelegraphNode {
    Text(String),
    Element {
        tag: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        attrs: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        children: Option<Vec<TelegraphNode>>,
    },
}

#[derive(Deserialize)]
struct TelegraphResponse<T> {
    ok: bool,
    error: Option<String>,
    result: Option<T>,
}

#[derive(Deserialize)]
struct AccountResult {
    access_token: String,
}

#[derive(Deserialize)]
struct PageResult {
    url: String,
}

/// Ensures an active Telegraph access token exists, creating a new account if needed.
pub async fn get_or_create_access_token() -> Result<String> {
    let lock = get_token_lock();
    {
        let read = lock.read().await;
        if let Some(ref tok) = *read {
            return Ok(tok.clone());
        }
    }

    let mut write = lock.write().await;
    if let Some(ref tok) = *write {
        return Ok(tok.clone());
    }

    let (author_name, author_url) = get_author_info();
    let short_name = if author_name.len() > 32 {
        "KostubetBot".to_string()
    } else {
        author_name.clone()
    };

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.telegra.ph/createAccount")
        .json(&serde_json::json!({
            "short_name": short_name,
            "author_name": author_name,
            "author_url": author_url
        }))
        .send()
        .await
        .context("Failed to connect to Telegraph API")?;

    let json: TelegraphResponse<AccountResult> = resp
        .json()
        .await
        .context("Invalid response from Telegraph createAccount")?;

    if !json.ok {
        let err = json.error.unwrap_or_else(|| "Unknown Telegraph error".to_string());
        return Err(anyhow::anyhow!("Telegraph createAccount error: {}", err));
    }

    let token = json
        .result
        .ok_or_else(|| anyhow::anyhow!("Missing Telegraph token in result"))?
        .access_token;

    *write = Some(token.clone());
    Ok(token)
}

/// Publishes a text guide to Telegraph and returns the article URL with Instant View support.
pub async fn publish_text_guide(
    title: &str,
    author: Option<&str>,
    text: &str,
) -> Result<String> {
    let token = get_or_create_access_token().await?;

    let mut nodes = Vec::new();
    for paragraph in text.split('\n') {
        let trimmed = paragraph.trim();
        if !trimmed.is_empty() {
            // Check if paragraph looks like a header (starts with # or ##)
            if let Some(h3) = trimmed.strip_prefix("### ") {
                nodes.push(TelegraphNode::Element {
                    tag: "h4".to_string(),
                    attrs: None,
                    children: Some(vec![TelegraphNode::Text(h3.trim().to_string())]),
                });
            } else if let Some(h2) = trimmed.strip_prefix("## ") {
                nodes.push(TelegraphNode::Element {
                    tag: "h4".to_string(),
                    attrs: None,
                    children: Some(vec![TelegraphNode::Text(h2.trim().to_string())]),
                });
            } else if let Some(h1) = trimmed.strip_prefix("# ") {
                nodes.push(TelegraphNode::Element {
                    tag: "h3".to_string(),
                    attrs: None,
                    children: Some(vec![TelegraphNode::Text(h1.trim().to_string())]),
                });
            } else {
                nodes.push(TelegraphNode::Element {
                    tag: "p".to_string(),
                    attrs: None,
                    children: Some(vec![TelegraphNode::Text(trimmed.to_string())]),
                });
            }
        }
    }

    if nodes.is_empty() {
        nodes.push(TelegraphNode::Element {
            tag: "p".to_string(),
            attrs: None,
            children: Some(vec![TelegraphNode::Text(text.to_string())]),
        });
    }

    // Append author attribution at bottom if submitted by user
    if let Some(author_user) = author {
        let clean = author_user.trim().trim_start_matches('@');
        if !clean.is_empty() {
            nodes.push(TelegraphNode::Element {
                tag: "p".to_string(),
                attrs: None,
                children: Some(vec![TelegraphNode::Text(format!("👤 Руководство подготовил: @{}", clean))]),
            });
        }
    }

    create_page(&token, title, &nodes).await
}

/// Resolves the official Telegraph author name and URL from environment variables.
fn get_author_info() -> (String, String) {
    let author_name = std::env::var("TELEGRAPH_AUTHOR_NAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Kostubet".to_string());

    let author_url = std::env::var("TELEGRAPH_AUTHOR_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://t.me".to_string());

    (author_name, author_url)
}

/// Internal helper to call `createPage` on Telegraph API.
/// Author and URL are resolved from TELEGRAPH_AUTHOR_NAME and TELEGRAPH_AUTHOR_URL env variables.
async fn create_page(
    token: &str,
    title: &str,
    content: &[TelegraphNode],
) -> Result<String> {
    let safe_title = if title.trim().is_empty() {
        "Руководство и настройка"
    } else {
        title.trim()
    };

    let (author_name, author_url) = get_author_info();

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.telegra.ph/createPage")
        .json(&serde_json::json!({
            "access_token": token,
            "title": safe_title,
            "author_name": author_name,
            "author_url": author_url,
            "content": content,
            "return_content": false
        }))
        .send()
        .await
        .context("Failed to send createPage request to Telegraph")?;

    let json: TelegraphResponse<PageResult> = resp
        .json()
        .await
        .context("Invalid response from Telegraph createPage")?;

    if !json.ok {
        let err = json.error.unwrap_or_else(|| "Unknown error".to_string());
        return Err(anyhow::anyhow!("Telegraph createPage error: {}", err));
    }

    let page_url = json
        .result
        .ok_or_else(|| anyhow::anyhow!("Missing result in Telegraph response"))?
        .url;

    Ok(page_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_telegraph_nodes_serialization() {
        let nodes = vec![
            TelegraphNode::Element {
                tag: "h4".to_string(),
                attrs: None,
                children: Some(vec![TelegraphNode::Text("Шаг 1".to_string())]),
            },
            TelegraphNode::Element {
                tag: "p".to_string(),
                attrs: None,
                children: Some(vec![TelegraphNode::Text("Установите приложение.".to_string())]),
            },
        ];

        let json = serde_json::to_string(&nodes).unwrap();
        assert!(json.contains("\"tag\":\"h4\""));
        assert!(json.contains("Шаг 1"));
        assert!(json.contains("\"tag\":\"p\""));
    }
}
