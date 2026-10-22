//! Input validation shared by server and clients.

use crate::limits::*;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error(
        "must be {USERNAME_MIN_CHARS}-{USERNAME_MAX_CHARS} characters of a-z, 0-9 or underscore"
    )]
    Username,
    #[error(
        "must be at least {PASSWORD_MIN_CHARS} characters and at most {PASSWORD_MAX_BYTES} bytes"
    )]
    Password,
    #[error("must be at most {max} characters with no control characters")]
    Text { max: usize },
}

/// Normalizes and validates a login name or persona handle.
///
/// Handles are ASCII-only on purpose: it rules out lookalike (homoglyph)
/// impersonation and keeps WebFinger addresses unambiguous. Input is
/// lowercased; anything outside `[a-z0-9_]` is rejected rather than folded.
pub fn username(input: &str) -> Result<String, ValidationError> {
    let lowered = input.trim().to_ascii_lowercase();
    let len = lowered.chars().count();
    let ok = (USERNAME_MIN_CHARS..=USERNAME_MAX_CHARS).contains(&len)
        && lowered
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(lowered)
    } else {
        Err(ValidationError::Username)
    }
}

/// Checks password length. Passwords are not normalized or trimmed.
pub fn password(input: &str) -> Result<(), ValidationError> {
    if input.chars().count() >= PASSWORD_MIN_CHARS && input.len() <= PASSWORD_MAX_BYTES {
        Ok(())
    } else {
        Err(ValidationError::Password)
    }
}

/// Validates optional free text such as a display name or summary. Returns
/// `None` for absent or blank input.
pub fn optional_text(input: Option<&str>, max: usize) -> Result<Option<String>, ValidationError> {
    let Some(raw) = input else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let has_control = trimmed
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t');
    if has_control || trimmed.chars().count() > max {
        return Err(ValidationError::Text { max });
    }
    Ok(Some(trimmed.to_owned()))
}

pub fn display_name(input: Option<&str>) -> Result<Option<String>, ValidationError> {
    let value = optional_text(input, DISPLAY_NAME_MAX_CHARS)?;
    if value.as_deref().is_some_and(|v| v.contains('\n')) {
        return Err(ValidationError::Text {
            max: DISPLAY_NAME_MAX_CHARS,
        });
    }
    Ok(value)
}

pub fn summary(input: Option<&str>) -> Result<Option<String>, ValidationError> {
    optional_text(input, SUMMARY_MAX_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames() {
        assert_eq!(username("  InkFox_9 ").unwrap(), "inkfox_9");
        assert!(username("").is_err());
        assert!(username("a".repeat(31).as_str()).is_err());
        assert!(username("ink-fox").is_err());
        assert!(username("ink fox").is_err());
        // Cyrillic 'а' looks like Latin 'a' and must be rejected, not folded.
        assert!(username("\u{0430}lice").is_err());
        assert!(username("ﬁsh").is_err());
    }

    #[test]
    fn passwords() {
        assert!(password("short").is_err());
        assert!(password("twelve chars").is_ok());
        assert!(password(&"x".repeat(PASSWORD_MAX_BYTES + 1)).is_err());
        // 12 multi-byte characters count as 12, not 36.
        assert!(password(&"é".repeat(12)).is_ok());
    }

    #[test]
    fn text_fields() {
        assert_eq!(display_name(None).unwrap(), None);
        assert_eq!(display_name(Some("   ")).unwrap(), None);
        assert_eq!(display_name(Some(" Ink ")).unwrap().as_deref(), Some("Ink"));
        assert!(display_name(Some("two\nlines")).is_err());
        assert!(display_name(Some("bell\u{7}")).is_err());
        assert!(summary(Some("line one\nline two")).is_ok());
        assert!(summary(Some(&"s".repeat(SUMMARY_MAX_CHARS + 1))).is_err());
    }
}
