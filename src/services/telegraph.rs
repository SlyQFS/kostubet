//! Telegraph API client and Instant View guide publisher.
//!
//! Provides automated article creation on `telegra.ph` with Instant View support
//! from both simple text guides and step-by-step illustrated walkthroughs.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use teloxide::net::Download;
use teloxide::prelude::*;

static TELEGRAPH_ACCESS_TOKEN: OnceLock<tokio::sync::RwLock<Option<String>>> = OnceLock::new();

fn get_token_lock() -> &'static tokio::sync::RwLock<Option<String>> {
    TELEGRAPH_ACCESS_TOKEN.get_or_init(|| tokio::sync::RwLock::new(None))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GuideStep {
    pub text: String,
    pub photo_file_id: Option<String>,
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

#[derive(Deserialize)]
struct UploadResultItem {
    src: Option<String>,
    error: Option<String>,
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

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.telegra.ph/createAccount")
        .json(&serde_json::json!({
            "short_name": "KostubetBot",
            "author_name": "Kostubet Community",
            "author_url": "https://t.me"
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

/// Uploads an image downloaded from Telegram to Telegraph's CDN (`telegra.ph/upload`).
/// Returns the full image URL (e.g. `https://telegra.ph/file/...`).
pub async fn upload_image_to_telegraph(bot: &Bot, file_id: &str) -> Result<String> {
    let file = bot
        .get_file(file_id.to_string())
        .await
        .context("Failed to get Telegram file info")?;

    let mut image_bytes = Vec::new();
    bot.download_file(&file.path, &mut image_bytes)
        .await
        .context("Failed to download image bytes from Telegram")?;

    let part = reqwest::multipart::Part::bytes(image_bytes)
        .file_name("screenshot.jpg")
        .mime_str("image/jpeg")?;

    let form = reqwest::multipart::Form::new().part("file", part);

    let client = reqwest::Client::new();
    let resp = client
        .post("https://telegra.ph/upload")
        .multipart(form)
        .send()
        .await
        .context("Failed to upload image to Telegraph")?;

    let upload_items: Vec<UploadResultItem> = resp
        .json()
        .await
        .context("Invalid JSON from Telegraph upload")?;

    if let Some(first) = upload_items.first() {
        if let Some(ref src) = first.src {
            let full_url = if src.starts_with("http") {
                src.clone()
            } else {
                format!("https://telegra.ph{}", src)
            };
            return Ok(full_url);
        } else if let Some(ref err) = first.error {
            return Err(anyhow::anyhow!("Telegraph upload error: {}", err));
        }
    }

    Err(anyhow::anyhow!("Empty upload response from Telegraph"))
}

/// Publishes a quick text guide to Telegraph and returns the article URL.
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
            nodes.push(TelegraphNode::Element {
                tag: "p".to_string(),
                attrs: None,
                children: Some(vec![TelegraphNode::Text(trimmed.to_string())]),
            });
        }
    }

    if nodes.is_empty() {
        nodes.push(TelegraphNode::Element {
            tag: "p".to_string(),
            attrs: None,
            children: Some(vec![TelegraphNode::Text(text.to_string())]),
        });
    }

    create_page(&token, title, author, &nodes).await
}

/// Publishes a structured step-by-step illustrated guide with screenshots to Telegraph.
pub async fn publish_steps_guide(
    bot: &Bot,
    title: &str,
    author: Option<&str>,
    steps: &[GuideStep],
) -> Result<String> {
    let token = get_or_create_access_token().await?;
    let mut nodes = Vec::new();

    for (i, step) in steps.iter().enumerate() {
        let step_num = i + 1;
        // Step Header
        nodes.push(TelegraphNode::Element {
            tag: "h4".to_string(),
            attrs: None,
            children: Some(vec![TelegraphNode::Text(format!("Шаг {}", step_num))]),
        });

        // Step Screenshot if present
        if let Some(ref fid) = step.photo_file_id {
            match upload_image_to_telegraph(bot, fid).await {
                Ok(img_url) => {
                    nodes.push(TelegraphNode::Element {
                        tag: "figure".to_string(),
                        attrs: None,
                        children: Some(vec![TelegraphNode::Element {
                            tag: "img".to_string(),
                            attrs: Some(serde_json::json!({ "src": img_url })),
                            children: None,
                        }]),
                    });
                }
                Err(e) => {
                    tracing::warn!("Failed to upload step {} image to Telegraph: {}", step_num, e);
                }
            }
        }

        // Step Text / Explanation
        if !step.text.trim().is_empty() {
            nodes.push(TelegraphNode::Element {
                tag: "p".to_string(),
                attrs: None,
                children: Some(vec![TelegraphNode::Text(step.text.trim().to_string())]),
            });
        }
    }

    if nodes.is_empty() {
        nodes.push(TelegraphNode::Element {
            tag: "p".to_string(),
            attrs: None,
            children: Some(vec![TelegraphNode::Text("Инструкция отсутствует.".to_string())]),
        });
    }

    create_page(&token, title, author, &nodes).await
}

/// Internal helper to call `createPage` on Telegraph API.
async fn create_page(
    token: &str,
    title: &str,
    author: Option<&str>,
    content: &[TelegraphNode],
) -> Result<String> {
    let safe_title = if title.trim().is_empty() {
        "Руководство и настройка"
    } else {
        title.trim()
    };

    let author_name = author.unwrap_or("Kostubet Community");

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.telegra.ph/createPage")
        .json(&serde_json::json!({
            "access_token": token,
            "title": safe_title,
            "author_name": author_name,
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
