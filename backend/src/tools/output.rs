use crate::{mcp::MAX_TEXT, panel::Reply};

/// Truncates at a character boundary so the result is always valid UTF-8.
pub fn truncate(text: &str) -> String {
    if text.len() <= MAX_TEXT {
        return text.to_string();
    }

    let mut end = MAX_TEXT;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    format!(
        "{}\n[Truncated: showing the first {end} of {} bytes…]",
        &text[..end],
        text.len()
    )
}

pub fn render(reply: &Reply) -> String {
    if reply.status >= 400 {
        let body = String::from_utf8_lossy(&reply.body);
        return truncate(&format!("HTTP {}: {}", reply.status, body.trim()));
    }

    let content_type = reply.content_type.as_deref().unwrap_or_default();

    match std::str::from_utf8(&reply.body) {
        Ok(text) if is_text(content_type) => truncate(text),
        _ => binary_note(content_type, reply.body.len()),
    }
}

pub fn binary_note(content_type: &str, len: usize) -> String {
    let content_type = if content_type.is_empty() {
        "unknown content type"
    } else {
        content_type
    };

    format!("Binary response ({content_type}, {len} bytes)")
}

fn is_text(content_type: &str) -> bool {
    content_type.is_empty()
        || content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("javascript")
        || content_type.contains("x-www-form-urlencoded")
}

/// Strips ANSI escapes and carriage returns so console output is readable as plain text.
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '\u{1b}' => match chars.peek() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\u{07}' || next == '\u{1b}' {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {}
            character => out.push(character),
        }
    }

    out
}
