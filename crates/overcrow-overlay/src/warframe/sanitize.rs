//! Text sanitization for display and clipboard output.

use super::model::{STRING_MAX_CHARS, bound_chars};

/// Strip control characters and path-ish bytes from free-form API strings.
pub fn sanitize_display(input: &str, max_chars: usize) -> String {
    let cleaned: String = input.chars().filter(|c| !c.is_control()).collect();
    bound_chars(cleaned.trim(), max_chars)
}

/// Player names for `/w` must not inject newlines or extra slash commands.
pub fn sanitize_player_name(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
        .take(32)
        .collect();
    cleaned.trim().to_owned()
}

pub fn sanitize_item_name(input: &str) -> String {
    sanitize_display(input, STRING_MAX_CHARS)
}

/// Clipboard payloads stay single-line and bounded.
pub fn sanitize_clipboard_text(input: &str) -> Result<String, String> {
    let cleaned: String = input
        .chars()
        .filter(|c| *c != '\0')
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return Err("empty text".to_owned());
    }
    if trimmed.chars().count() > 512 {
        return Err("text too long".to_owned());
    }
    Ok(trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{sanitize_clipboard_text, sanitize_player_name};

    #[test]
    fn player_names_cannot_inject_commands() {
        assert_eq!(
            sanitize_player_name("Nice/Player\n/w evil"),
            "NicePlayerw evil"
        );
        assert_eq!(sanitize_player_name("  Alice  "), "Alice");
        assert!(!sanitize_player_name("x\ny").contains('\n'));
    }

    #[test]
    fn clipboard_rejects_empty_and_nulls() {
        assert!(sanitize_clipboard_text("").is_err());
        assert!(sanitize_clipboard_text("\0\0").is_err());
        assert_eq!(
            sanitize_clipboard_text("/w Bob Hi,\nWTB X for 1p").unwrap(),
            "/w Bob Hi, WTB X for 1p"
        );
    }
}
