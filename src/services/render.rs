//! Markdown rendering, HTML formatting, card construction, and Telegram message dispatch.
//!
//! Provides utilities to convert GitHub markdown into Telegram-compatible HTML,
//! assemble release cards (`PostData`), build download keyboards, and dispatch
//! cards to Telegram with granular retry handling on rate-limiting.

use html_escape::encode_text;
use teloxide::prelude::*;
use teloxide::types::{
    ChatAction, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, LinkPreviewOptions,
    MessageId, ParseMode, ThreadId,
};

#[derive(Debug, Clone)]
pub enum DownloadTarget {
    Url(String),
    CallbackData(String),
}

#[derive(Debug, Clone)]
pub struct PostData {
    pub title: String,
    pub body: Option<String>,
    pub diff_url: Option<String>,
    pub tags: Vec<String>,
    pub cover_image: Option<String>, // file_id or URL
    pub download_buttons: Vec<(String, DownloadTarget)>,
}

/// Helper function to construct unified `PostData` for custom APK releases.
pub fn build_apk_post_data(
    app_name: &str,
    version: &str,
    changelog: Option<String>,
    diff_url: Option<String>,
    cover_image: Option<String>,
    tags: Vec<String>,
    apk_files: &[(i64, String)], // (file_row_id, variant_label)
) -> PostData {
    PostData {
        title: format!("{} v{}", app_name, version),
        body: changelog,
        diff_url,
        tags,
        cover_image,
        download_buttons: apk_files
            .iter()
            .map(|(id, variant)| {
                (
                    format!("⬇️ Скачать ({})", variant),
                    DownloadTarget::CallbackData(format!("apk_get:{}", id)),
                )
            })
            .collect(),
    }
}

pub fn disabled_link_preview() -> LinkPreviewOptions {
    LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    }
}

/// Convert basic Markdown syntax to Telegram-compatible HTML.
///
/// Consecutively occurring empty lines in markdown (common in GitHub auto-generated changelogs)
/// are collapsed into a single empty line to prevent bloated spacing inside expandable blockquotes.
pub fn markdown_to_telegram_html(md: &str) -> String {
    let mut result = String::new();
    let mut in_code_block = false;
    let mut last_line_blank = false;

    for line in md.lines() {
        let trimmed = line.trim();

        // Code block toggle (```)
        if trimmed.starts_with("```") {
            if in_code_block {
                result.push_str("</code></pre>\n");
                in_code_block = false;
            } else {
                result.push_str("<pre><code>");
                in_code_block = true;
            }
            last_line_blank = false;
            continue;
        }

        if in_code_block {
            result.push_str(&encode_text(line));
            result.push('\n');
            continue;
        }

        if trimmed.is_empty() {
            if !last_line_blank {
                result.push('\n');
            }
            last_line_blank = true;
            continue;
        }
        last_line_blank = false;

        if trimmed.starts_with('#') {
            let header_text = trimmed.trim_start_matches('#').trim();
            result.push_str(&format!("<b>{}</b>\n", encode_text(header_text)));
        } else if trimmed.starts_with("- ")
            || trimmed.starts_with("+ ")
            || trimmed.starts_with("* ")
        {
            let list_item = &trimmed[2..];
            result.push_str(&format!("• {}\n", parse_inline_markdown(list_item)));
        } else {
            result.push_str(&parse_inline_markdown(line));
            result.push('\n');
        }
    }

    if in_code_block {
        result.push_str("</code></pre>");
    }

    result.trim_end().to_string()
}

/// Parse inline Markdown elements: **bold**, *italic*, `code`, [link](url)
fn parse_inline_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        // 1. Link: [text](url)
        if rest.starts_with('[') {
            if let Some(close_bracket) = rest.find(']') {
                let link_text = &rest[1..close_bracket];
                let remainder = &rest[close_bracket + 1..];
                if remainder.starts_with('(') {
                    if let Some(close_paren) = remainder.find(')') {
                        let url = &remainder[1..close_paren];
                        out.push_str(&format!(
                            "<a href=\"{}\">{}</a>",
                            encode_text(url),
                            encode_text(link_text)
                        ));
                        rest = &remainder[close_paren + 1..];
                        continue;
                    }
                }
            }
        }

        // 2. Bold: **text**
        if rest.starts_with("**") && rest.len() >= 4 {
            if let Some(close_bold) = rest[2..].find("**") {
                let inner = &rest[2..2 + close_bold];
                out.push_str(&format!("<b>{}</b>", encode_text(inner)));
                rest = &rest[2 + close_bold + 2..];
                continue;
            }
        }

        // 3. Inline Code: `code`
        if rest.starts_with('`') && rest.len() >= 2 {
            if let Some(close_code) = rest[1..].find('`') {
                let inner = &rest[1..1 + close_code];
                out.push_str(&format!("<code>{}</code>", encode_text(inner)));
                rest = &rest[1 + close_code + 1..];
                continue;
            }
        }

        // 4. Italic: *text* or _text_ (word-bounded only for `_`:
        // snake_case identifiers must stay literal).
        if (rest.starts_with('*') || rest.starts_with('_')) && rest.len() >= 2 {
            let delim = &rest[..1];
            let word_inner = delim == "_" && out.chars().last().is_some_and(|c| c.is_alphanumeric());
            if !word_inner {
                if let Some(close_italic) = rest[1..].find(delim) {
                    let inner = &rest[1..1 + close_italic];
                    if !inner.contains('\n') && !inner.is_empty() {
                        out.push_str(&format!("<i>{}</i>", encode_text(inner)));
                        rest = &rest[1 + close_italic + 1..];
                        continue;
                    }
                }
            }
        }

        // 5. Normal character
        let next_char = rest.chars().next().unwrap();
        let char_len = next_char.len_utf8();
        match next_char {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            _ => out.push(next_char),
        }
        rest = &rest[char_len..];
    }

    out
}

/// Removes `![alt](url)` markdown images (they cannot be displayed inside
/// Telegram text and previously rendered as broken `!<a>…</a>` artifacts).
fn remove_markdown_images(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;

    while let Some(pos) = rest.find("![") {
        let after = &rest[pos + 2..];
        if let Some(close_bracket) = after.find(']') {
            let remainder = &after[close_bracket + 1..];
            if remainder.starts_with('(') {
                if let Some(close_paren) = remainder.find(')') {
                    out.push_str(&rest[..pos]);
                    rest = &remainder[close_paren + 1..];
                    continue;
                }
            }
        }
        // Not a well-formed image — emit the '!' literally and move on.
        out.push('!');
        rest = &rest[pos + 1..];
    }

    out.push_str(rest);
    out
}

/// Returns true when `tag` (the text between `<` and `>`) looks like a real
/// HTML/XML tag rather than a "less than" comparison (`a < b`).
fn looks_like_tag(tag: &str) -> bool {
    let t = tag.trim();
    if t.is_empty() {
        return false;
    }
    let name = t.trim_start_matches('/');
    if name.is_empty() {
        return false;
    }
    let mut parts = name.split_whitespace();
    let tag_name = match parts.next() {
        Some(n) => n.trim_end_matches('/'),
        None => return false,
    };
    if tag_name.is_empty()
        || !tag_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ':' || c == '_' || c == '.')
    {
        return false;
    }
    // Attribute chunks must not contain angle brackets.
    parts.all(|p| !p.contains('<') && !p.contains('>'))
}

/// Strips inline HTML/XML tags while keeping their inner text
/// (`<details>`-style wrappers). `<br>` becomes a line break.
fn strip_html_tags(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;

    while let Some(pos) = rest.find('<') {
        let after = &rest[pos + 1..];
        let tag_like_start = after.starts_with('/')
            || after.starts_with('!')
            || after.starts_with(|c: char| c.is_ascii_alphabetic());

        if tag_like_start {
            if let Some(gt) = after.find('>') {
                let tag = &after[..gt];
                if !tag.contains('<') && looks_like_tag(tag) {
                    out.push_str(&rest[..pos]);
                    let name = tag.trim().trim_start_matches('/').trim();
                    if name.starts_with("br") {
                        out.push('\n');
                    }
                    rest = &after[gt + 1..];
                    continue;
                }
            }
        }

        // Literal '<' (e.g. a comparison) — keep it raw; the markdown
        // renderer escapes < > & itself (escaping here would double-escape).
        out.push_str(&rest[..pos]);
        out.push('<');
        rest = &rest[pos + 1..];
    }

    out.push_str(rest);
    out
}

/// Preprocesses raw changelog markdown before rendering:
/// removes markdown images, HTML comments (incl. multi-line), and
/// strips HTML/XML tags outside fenced code blocks.
pub fn clean_markdown_source(md: &str) -> String {
    let mut out_lines: Vec<String> = Vec::new();
    let mut in_code_block = false;
    let mut in_comment = false;

    for line in md.lines() {
        let mut work = line.to_string();

        if in_code_block {
            if work.trim_start().starts_with("```") {
                in_code_block = false;
            }
            out_lines.push(work);
            continue;
        }
        if work.trim_start().starts_with("```") {
            in_code_block = true;
            out_lines.push(work);
            continue;
        }

        // HTML comments, possibly spanning multiple lines.
        if in_comment {
            if let Some(end) = work.find("-->") {
                work = work[end + 3..].to_string();
                in_comment = false;
            } else {
                out_lines.push(String::new());
                continue;
            }
        }
        while let Some(start) = work.find("<!--") {
            let after = &work[start + 4..];
            match after.find("-->") {
                Some(end_rel) => {
                    work = format!("{}{}", &work[..start], &after[end_rel + 3..]);
                }
                None => {
                    work.truncate(start);
                    in_comment = true;
                    break;
                }
            }
        }

        work = remove_markdown_images(&work);
        work = strip_html_tags(&work);

        out_lines.push(work);
    }

    out_lines.join("\n")
}

/// Renders markdown into Telegram HTML, guaranteeing the result fits into
/// `max_chars` characters. Truncation is applied to the *source* markdown
/// (at word boundaries), never to the rendered HTML — so the output always
/// contains balanced tags and valid entities.
fn render_body_limited(md: &str, max_chars: usize) -> (String, bool) {
    let mut src: String = md.to_string();
    let mut did_cut = false;

    loop {
        let rendered = markdown_to_telegram_html(&src);
        if rendered.chars().count() <= max_chars || src.is_empty() {
            return (rendered, did_cut);
        }

        // Estimate a source cut so the rendered result fits, with a margin.
        let ratio = max_chars.saturating_mul(100) / rendered.chars().count().max(1);
        let src_len = src.chars().count();
        let mut cut = (src_len * ratio / 100).max(100).min(src_len);
        if cut >= src_len {
            cut = src_len.saturating_sub(100);
        }

        let mut truncated: String = src.chars().take(cut).collect();
        // Roll back to a whitespace boundary so no word/markdown token is split.
        while truncated
            .chars()
            .last()
            .is_some_and(|c| !c.is_whitespace())
            && truncated.chars().count() > 50
        {
            truncated.pop();
        }
        truncated = truncated.trim_end().to_string();
        if truncated.is_empty() {
            return (rendered, true);
        }
        did_cut = true;
        src = truncated;
    }
}

// Order of card sections:
// 1. Title (🆕 <b>...</b>)
// 2. Body (<blockquote expandable>...</blockquote>)
// 3. Diff URL (🔗 <a href="...">...</a>)
// 4. Tags (#tag1 #tag2)
// 5. Download buttons (InlineKeyboardMarkup)
pub fn render_post_text(post: &PostData) -> String {
    let mut card = format!("🆕 <b>{}</b>", encode_text(&post.title));

    if let Some(ref body) = post.body {
        let trimmed = body.trim();
        if !trimmed.is_empty() {
            // Drop images / raw HTML / XML noise before rendering.
            let cleaned = clean_markdown_source(trimmed);
            let cleaned_trim = cleaned.trim();
            if !cleaned_trim.is_empty() {
                let max_body_len = 3000;
                let (rich_body, was_truncated) = render_body_limited(cleaned_trim, max_body_len);
                let body_content = if was_truncated && !rich_body.is_empty() {
                    format!("{}\n\n<i>... [описание обрезано]</i>", rich_body)
                } else {
                    rich_body
                };

                card.push_str("\n\n<blockquote expandable>");
                card.push_str(&body_content);
                card.push_str("</blockquote>");
            }
        }
    }

    if let Some(ref diff) = post.diff_url {
        let trimmed_diff = diff.trim();
        if !trimmed_diff.is_empty() {
            card.push_str(&format!(
                "\n\n🔗 <a href=\"{}\"><b>Открыть список изменений на GitHub</b></a>",
                encode_text(trimmed_diff)
            ));
        }
    }

    if !post.tags.is_empty() {
        let tag_line: Vec<String> = post
            .tags
            .iter()
            .map(|t| format!("#{}", encode_text(t.trim().trim_start_matches('#'))))
            .collect();
        card.push_str(&format!("\n\n{}", tag_line.join(" ")));
    }

    card
}

pub fn render_post_keyboard(post: &PostData) -> Option<InlineKeyboardMarkup> {
    if post.download_buttons.is_empty() {
        return None;
    }

    let mut rows = Vec::new();
    for (label, target) in &post.download_buttons {
        let btn = match target {
            DownloadTarget::Url(url) => InlineKeyboardButton::url(
                label.clone(),
                reqwest::Url::parse(url)
                    .unwrap_or_else(|_| reqwest::Url::parse("https://github.com").unwrap()),
            ),
            DownloadTarget::CallbackData(data) => {
                InlineKeyboardButton::callback(label.clone(), data.clone())
            }
        };
        rows.push(vec![btn]);
    }

    Some(InlineKeyboardMarkup::new(rows))
}

/// Executes a single Telegram API call with backoff retry on `RequestError::RetryAfter`.
async fn execute_telegram_with_retry<F, Fut, T>(mut f: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, teloxide::RequestError>>,
{
    let mut retries = 0;
    const MAX_RETRIES: u32 = 3;

    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(teloxide::RequestError::RetryAfter(seconds)) => {
                if retries >= MAX_RETRIES {
                    return Err(anyhow::anyhow!(
                        "Telegram RateLimit exceeded maximum retries"
                    ));
                }
                tracing::warn!(
                    "Telegram rate limit hit! Waiting {:?} seconds before retry (attempt {}/{})",
                    seconds,
                    retries + 1,
                    MAX_RETRIES
                );
                tokio::time::sleep(seconds.duration()).await;
                retries += 1;
            }
            Err(err) => return Err(err.into()),
        }
    }
}

pub async fn send_post(
    bot: &Bot,
    chat_id: i64,
    thread_id: Option<i64>,
    post: &PostData,
) -> anyhow::Result<Message> {
    // Send typing action to make bot feel alive on longer requests
    let _ = bot
        .send_chat_action(ChatId(chat_id), ChatAction::Typing)
        .await;

    let text = render_post_text(post);
    let kb = render_post_keyboard(post);

    // Photo card attempt. A failed photo send (e.g. a stale cover file_id
    // after a bot token change, or an unreadable remote URL) must NOT kill
    // the whole publication: log it and fall back to the text-only card.
    if let Some(ref cover) = post.cover_image {
        let photo_result = match build_cover_input_file(cover) {
            Some(input_file) => {
                send_photo_card(bot, chat_id, thread_id, &input_file, &text, &post.title, kb.as_ref())
                    .await
            }
            None => Err(anyhow::anyhow!("invalid cover reference")),
        };

        if let Ok(msg) = photo_result {
            return Ok(msg);
        } else if let Err(e) = photo_result {
            tracing::warn!(
                "Photo card send failed ({}); falling back to text-only card",
                e
            );
        }
    }

    send_text_card(bot, chat_id, thread_id, &text, kb).await
}

/// Builds a Telegram input file from a cover reference (URL or bot file_id).
fn build_cover_input_file(cover: &str) -> Option<InputFile> {
    if cover.starts_with("http://") || cover.starts_with("https://") {
        reqwest::Url::parse(cover).ok().map(InputFile::url)
    } else {
        Some(InputFile::file_id(cover.to_string()))
    }
}

/// Sends a photo card: photo+caption when the caption fits 1024 chars,
/// otherwise photo with a short header followed by a full text message.
async fn send_photo_card(
    bot: &Bot,
    chat_id: i64,
    thread_id: Option<i64>,
    input_file: &InputFile,
    text: &str,
    title: &str,
    kb: Option<&InlineKeyboardMarkup>,
) -> anyhow::Result<Message> {
    if text.chars().count() <= 1024 {
        let bot_clone = bot.clone();
        let input_file_clone = input_file.clone();
        let text_clone = text.to_string();
        let kb_clone = kb.cloned();

        let msg = execute_telegram_with_retry(|| {
            let mut req = bot_clone
                .send_photo(ChatId(chat_id), input_file_clone.clone())
                .caption(text_clone.clone())
                .parse_mode(ParseMode::Html);

            if let Some(tid) = thread_id {
                req = req.message_thread_id(ThreadId(MessageId(tid as i32)));
            }
            if let Some(ref keyboard) = kb_clone {
                req = req.reply_markup(keyboard.clone());
            }
            req.send()
        })
        .await?;

        return Ok(msg);
    }

    // Caption > 1024: send the photo with a short header, then a separate
    // text message with the full card and the keyboard.
    let short_caption = format!("🆕 <b>{}</b>", encode_text(title));
    let bot_clone = bot.clone();
    let input_file_clone = input_file.clone();

    let _photo_msg = execute_telegram_with_retry(|| {
        let mut photo_req = bot_clone
            .send_photo(ChatId(chat_id), input_file_clone.clone())
            .caption(short_caption.clone())
            .parse_mode(ParseMode::Html);

        if let Some(tid) = thread_id {
            photo_req = photo_req.message_thread_id(ThreadId(MessageId(tid as i32)));
        }
        photo_req.send()
    })
    .await?;

    send_text_card(bot, chat_id, thread_id, text, kb.cloned()).await
}

/// Sends the text card (with the download keyboard attached).
async fn send_text_card(
    bot: &Bot,
    chat_id: i64,
    thread_id: Option<i64>,
    text: &str,
    kb: Option<InlineKeyboardMarkup>,
) -> anyhow::Result<Message> {
    let bot_clone = bot.clone();
    let text_clone = text.to_string();

    let msg = execute_telegram_with_retry(|| {
        let mut req = bot_clone
            .send_message(ChatId(chat_id), text_clone.clone())
            .parse_mode(ParseMode::Html)
            .link_preview_options(disabled_link_preview());

        if let Some(tid) = thread_id {
            req = req.message_thread_id(ThreadId(MessageId(tid as i32)));
        }
        if let Some(ref keyboard) = kb {
            req = req.reply_markup(keyboard.clone());
        }
        req.send()
    })
    .await?;

    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_post_text() {
        let post = PostData {
            title: "Tokio v1.40.0".to_string(),
            body: Some("Added **async** driver".to_string()),
            diff_url: Some(
                "https://github.com/tokio-rs/tokio/compare/v1.39.0...v1.40.0".to_string(),
            ),
            tags: vec!["async".to_string(), "rust".to_string()],
            cover_image: None,
            download_buttons: vec![(
                "⬇️ Скачать (universal)".to_string(),
                DownloadTarget::Url("https://example.com/app.apk".to_string()),
            )],
        };

        let rendered = render_post_text(&post);
        assert!(rendered.contains("🆕 <b>Tokio v1.40.0</b>"));
        assert!(rendered.contains("<blockquote expandable>Added <b>async</b> driver</blockquote>"));
        assert!(rendered.contains("#async #rust"));

        let kb = render_post_keyboard(&post);
        assert!(kb.is_some());
    }

    #[test]
    fn test_markdown_formatting_advanced() {
        let md = "# Major Release\n```rust\nfn main() { println!(\"<Hello & World>\"); }\n```\n- Feature: `speed` & [doc](https://example.com)\n*Italic* and **Bold**";
        let html = markdown_to_telegram_html(md);
        assert!(html.contains("<b>Major Release</b>"));
        assert!(html.contains(
            "<pre><code>fn main() { println!(\"&lt;Hello &amp; World&gt;\"); }\n</code></pre>"
        ));
        assert!(html.contains(
            "• Feature: <code>speed</code> &amp; <a href=\"https://example.com\">doc</a>"
        ));
        assert!(html.contains("<i>Italic</i>"));
        assert!(html.contains("<b>Bold</b>"));
    }

    #[test]
    fn test_collapse_consecutive_blank_lines_github_style() {
        let md = "## What's Changed\n* Bump version by @user in [PR #1](https://github.com/owner/repo/pull/1)\n\n\n**Full Changelog**: https://github.com/owner/repo/compare/v1.0.0...v1.0.1";
        let html = markdown_to_telegram_html(md);

        // Verify that consecutive 3 newlines are collapsed to a single empty line separator
        assert!(!html.contains("\n\n\n"));
        let expected = "<b>What's Changed</b>\n• Bump version by @user in <a href=\"https://github.com/owner/repo/pull/1\">PR #1</a>\n\n<b>Full Changelog</b>: https://github.com/owner/repo/compare/v1.0.0...v1.0.1";
        assert_eq!(html, expected);
    }

    #[test]
    fn test_build_apk_post_data_helper() {
        let post = build_apk_post_data(
            "V2RayNG",
            "1.8.5",
            Some("Fixed bugs".to_string()),
            Some("https://github.com/2dust/v2rayNG".to_string()),
            Some("file_img_123".to_string()),
            vec!["vpn".to_string(), "android".to_string()],
            &[(1, "arm64-v8a".to_string()), (2, "universal".to_string())],
        );

        assert_eq!(post.title, "V2RayNG v1.8.5");
        assert_eq!(post.tags, vec!["vpn", "android"]);
        assert_eq!(post.cover_image, Some("file_img_123".to_string()));
        assert_eq!(post.download_buttons.len(), 2);
        assert_eq!(post.download_buttons[0].0, "⬇️ Скачать (arm64-v8a)");
    }

    #[test]
    fn test_clean_strips_markdown_images() {
        let md = "Before ![logo](https://example.com/logo.png) after\n![only image](http://x/y.png)";
        let cleaned = clean_markdown_source(md);
        assert!(!cleaned.contains("!["), "images must be removed: {}", cleaned);
        assert!(!cleaned.contains("logo.png"));
        assert!(cleaned.contains("Before"));
        assert!(cleaned.contains("after"));
    }

    #[test]
    fn test_clean_strips_html_tags_keeps_text() {
        let md = "<details>\n<summary>New features</summary>\nFixed <b>crash</b> on start\n</details>";
        let cleaned = clean_markdown_source(md);
        assert!(!cleaned.contains('<'), "all tags must be stripped: {}", cleaned);
        assert!(cleaned.contains("New features"));
        assert!(cleaned.contains("Fixed crash on start"));
    }

    #[test]
    fn test_clean_removes_html_comments_multiline() {
        let md = "Keep this\n<!-- hidden comment\nspanning lines -->\nAnd this";
        let cleaned = clean_markdown_source(md);
        assert!(!cleaned.contains("hidden"));
        assert!(!cleaned.contains("<!--"));
        assert!(cleaned.contains("Keep this"));
        assert!(cleaned.contains("And this"));
    }

    #[test]
    fn test_clean_keeps_code_fences_untouched() {
        let md = "Text with <tag>\n```xml\n<manifest version=\"2\" />\n```\nEnd";
        let cleaned = clean_markdown_source(md);
        // Outside the fence tags are stripped; inside they survive verbatim.
        assert!(cleaned.contains("<manifest version=\\\"2\\\" />") || cleaned.contains("<manifest"));
        assert!(cleaned.lines().any(|l| l.starts_with("Text with")));
    }

    #[test]
    fn test_clean_br_becomes_newline_and_comparisons_kept() {
        let cleaned = clean_markdown_source("line1<br>line2 and a < b");
        assert!(cleaned.contains("line1\nline2"));
        assert!(cleaned.contains("a < b"), "comparisons must stay: {}", cleaned);
    }

    #[test]
    fn test_snake_case_not_italic() {
        let html = markdown_to_telegram_html("Use my_var_name carefully");
        assert!(!html.contains("<i>"), "snake_case must not become italic: {}", html);
        assert!(html.contains("my_var_name"));

        let italics = markdown_to_telegram_html("_real italic_ and *star*");
        assert!(italics.contains("<i>real italic</i>"));
        assert!(italics.contains("<i>star</i>"));
    }

    #[test]
    fn test_truncation_produces_valid_html_with_note() {
        let mut long_md = String::new();
        for i in 0..600 {
            long_md.push_str(&format!("- Item **{}** with `code` and [link](https://example.com/{}), plus some filling text here\n", i, i));
        }

        let post = PostData {
            title: "Big Release".to_string(),
            body: Some(long_md),
            diff_url: None,
            tags: vec![],
            cover_image: None,
            download_buttons: vec![],
        };

        let rendered = render_post_text(&post);
        assert!(rendered.contains("... [описание обрезано]"));

        // Extract the blockquote body and verify tag balance.
        let start = rendered.find("<blockquote").unwrap();
        let end = rendered.find("</blockquote>").unwrap();
        let body = &rendered[start..end];
        assert_eq!(
            body.matches("<b>").count(),
            body.matches("</b>").count(),
            "bold tags must be balanced after truncation"
        );
        assert_eq!(
            body.matches("<a href=").count(),
            body.matches("</a>").count(),
            "link tags must be balanced after truncation"
        );
        // No dangling cut-off tag at the very end of the body.
        let tail: String = body.chars().rev().take(5).collect();
        assert!(!tail.contains('"') || !body.trim_end().ends_with('='));
    }

    #[test]
    fn test_render_post_text_strips_images_from_body() {
        let post = PostData {
            title: "X".to_string(),
            body: Some("See ![screenshot](https://ex.com/1.png) changes".to_string()),
            diff_url: None,
            tags: vec![],
            cover_image: None,
            download_buttons: vec![],
        };
        let rendered = render_post_text(&post);
        assert!(!rendered.contains("screenshot"), "image alt must be gone: {}", rendered);
        assert!(rendered.contains("See"));
        assert!(rendered.contains("changes"));
    }

    #[test]
    fn test_cover_input_file() {
        assert!(build_cover_input_file("https://example.com/pic.jpg").is_some());
        assert!(build_cover_input_file("AgACAgIAAxkDAgMG").is_some()); // bot file_id
        assert!(build_cover_input_file("http://[invalid").is_none()); // malformed URL
    }
}
