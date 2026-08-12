use crate::github::{GithubUpdate, UpdateType};
use html_escape::encode_text;

/// Convert basic Markdown syntax (GitHub release notes) to Telegram-compatible HTML.
pub fn markdown_to_telegram_html(md: &str) -> String {
    let mut result = String::new();
    let mut in_code_block = false;

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
            continue;
        }

        if in_code_block {
            result.push_str(&encode_text(line));
            result.push('\n');
            continue;
        }

        if trimmed.starts_with('#') {
            let header_text = trimmed.trim_start_matches('#').trim();
            result.push_str(&format!("<b>{}</b>\n", encode_text(header_text)));
        } else if trimmed.starts_with("- ") || trimmed.starts_with("+ ") || trimmed.starts_with("* ") {
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

/// Parse inline Markdown elements safely on UTF-8 byte slices: **bold**, *italic*, `code`, [link](url)
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

        // 4. Italic: *text* or _text_
        if (rest.starts_with('*') || rest.starts_with('_')) && rest.len() >= 2 {
            let delim = &rest[..1];
            if let Some(close_italic) = rest[1..].find(delim) {
                let inner = &rest[1..1 + close_italic];
                if !inner.contains('\n') && !inner.is_empty() {
                    out.push_str(&format!("<i>{}</i>", encode_text(inner)));
                    rest = &rest[1 + close_italic + 1..];
                    continue;
                }
            }
        }

        // 5. Normal character: escape HTML special chars < > &
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

pub fn format_update_message(
    owner: &str,
    repo_name: &str,
    update: &GithubUpdate,
    is_dry_run: bool,
) -> String {
    let escaped_owner = encode_text(owner);
    let escaped_repo = encode_text(repo_name);
    let escaped_tag = encode_text(&update.tag_or_version);
    let escaped_title = encode_text(&update.title);
    let escaped_url = encode_text(&update.url);

    let prefix = if is_dry_run { "🧪 <b>[ТЕСТ / DRY-RUN]</b> " } else { "" };

    match update.update_type {
        UpdateType::Release => {
            let mut card = format!(
                "{}🚀 <b>НОВЫЙ РЕЛИЗ</b> • <b>{}/{}</b>\n\n\
                📦 <b>Версия:</b> <code>{}</code>\n\
                📌 <b>Заголовок:</b> {}\n\
                🔗 <a href=\"{}\"><b>Открыть список изменений на GitHub</b></a>",
                prefix, escaped_owner, escaped_repo, escaped_tag, escaped_title, escaped_url
            );

            if let Some(ref body) = update.body {
                let trimmed_body = body.trim();
                if !trimmed_body.is_empty() {
                    let rich_body = markdown_to_telegram_html(trimmed_body);
                    let max_body_len = 3200;
                    let body_content = if rich_body.chars().count() > max_body_len {
                        let cutoff: String = rich_body.chars().take(max_body_len).collect();
                        format!("{}\n\n<i>... [сообщение обрезано]</i>", cutoff)
                    } else {
                        rich_body
                    };

                    card.push_str("\n\n<blockquote expandable>");
                    card.push_str(&body_content);
                    card.push_str("</blockquote>");
                }
            }

            card
        }
        UpdateType::Tag => {
            format!(
                "{}🏷️ <b>НОВЫЙ ТЕГ</b> • <b>{}/{}</b>\n\n\
                🏷️ <b>Имя тега:</b> <code>{}</code>\n\
                🔗 <a href=\"{}\"><b>Просмотреть тег на GitHub</b></a>",
                prefix, escaped_owner, escaped_repo, escaped_tag, escaped_url
            )
        }
        UpdateType::Commit => {
            let mut card = format!(
                "{}📝 <b>НОВЫЙ КОММИТ</b> • <b>{}/{}</b>\n\n\
                🔑 <b>Хеш коммита:</b> <code>{}</code>\n\
                📌 <b>Описание:</b> {}\n\
                🔗 <a href=\"{}\"><b>Просмотреть коммит на GitHub</b></a>",
                prefix, escaped_owner, escaped_repo, escaped_tag, escaped_title, escaped_url
            );

            if let Some(ref body) = update.body {
                let trimmed_body = body.trim();
                if !trimmed_body.is_empty() {
                    let rich_body = markdown_to_telegram_html(trimmed_body);
                    let max_body_len = 3200;
                    let body_content = if rich_body.chars().count() > max_body_len {
                        let cutoff: String = rich_body.chars().take(max_body_len).collect();
                        format!("{}\n\n<i>... [сообщение обрезано]</i>", cutoff)
                    } else {
                        rich_body
                    };

                    card.push_str("\n\n<blockquote expandable>");
                    card.push_str(&body_content);
                    card.push_str("</blockquote>");
                }
            }

            card
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_to_telegram_html() {
        let md = "# Header\n- **Bold item** with `code` and [link](https://example.com)\n*Italic line*\nРусский текст c 🚀 эмодзи";
        let html = markdown_to_telegram_html(md);
        assert!(html.contains("<b>Header</b>"));
        assert!(html.contains("• <b>Bold item</b> with <code>code</code> and <a href=\"https://example.com\">link</a>"));
        assert!(html.contains("<i>Italic line</i>"));
        assert!(html.contains("Русский текст c 🚀 эмодзи"));
    }

    #[test]
    fn test_format_release_card() {
        let update = GithubUpdate {
            update_type: UpdateType::Release,
            id: "123".to_string(),
            tag_or_version: "v1.0.0".to_string(),
            title: "v1.0.0 Initial <Release> & Feature".to_string(),
            url: "https://github.com/foo/bar/releases/tag/v1.0.0".to_string(),
            body: Some("Added **foo** & *bar*".to_string()),
            etag: None,
            sha: None,
        };

        let msg = format_update_message("foo", "bar", &update, false);
        assert!(msg.contains("🚀 <b>НОВЫЙ РЕЛИЗ</b> • <b>foo/bar</b>"));
        assert!(msg.contains("<code>v1.0.0</code>"));
        assert!(msg.contains("<blockquote expandable>Added <b>foo</b> &amp; <i>bar</i></blockquote>"));
    }
}
