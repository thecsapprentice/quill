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
    #[error("{field} is required and must be at most {max} characters")]
    Required { field: &'static str, max: usize },
    #[error(
        "tag {0:?} must contain Latin letters or digits (accents are folded; other scripts can be used in a tag's display name only)"
    )]
    TagSlug(String),
    #[error("tag {0:?} must be written as namespace:name")]
    TagSpec(String),
    #[error("namespace must be 1-{NAMESPACE_MAX_CHARS} characters of a-z, 0-9 or hyphen")]
    Namespace,
    #[error("a prompt can have at most {PROMPT_MAX_TAGS} tags")]
    TooManyTags,
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

/// Required single-line text, such as a prompt title or tag display name.
pub fn required_line(
    field: &'static str,
    input: &str,
    max: usize,
) -> Result<String, ValidationError> {
    match optional_text(Some(input), max) {
        Ok(Some(v)) if !v.contains('\n') => Ok(v),
        _ => Err(ValidationError::Required { field, max }),
    }
}

pub fn prompt_title(input: &str) -> Result<String, ValidationError> {
    required_line("title", input, PROMPT_TITLE_MAX_CHARS)
}

/// Prompt bodies are Markdown; line breaks and tabs are allowed. Line endings
/// are normalized to `\n`.
pub fn prompt_body(input: &str) -> Result<String, ValidationError> {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    match optional_text(Some(&normalized), PROMPT_BODY_MAX_CHARS) {
        Ok(Some(v)) => Ok(v),
        _ => Err(ValidationError::Required {
            field: "body",
            max: PROMPT_BODY_MAX_CHARS,
        }),
    }
}

pub fn report_details(input: Option<&str>) -> Result<Option<String>, ValidationError> {
    optional_text(input, REPORT_DETAILS_MAX_CHARS)
}

pub fn tag_display(input: &str) -> Result<String, ValidationError> {
    required_line("tag name", input, TAG_DISPLAY_MAX_CHARS)
}

/// Validates a tag namespace name: lowercase ASCII letters, digits and
/// single hyphens.
pub fn namespace(input: &str) -> Result<String, ValidationError> {
    let name = input.trim().to_ascii_lowercase();
    let ok = (1..=NAMESPACE_MAX_CHARS).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--");
    if ok {
        Ok(name)
    } else {
        Err(ValidationError::Namespace)
    }
}

/// Letters that Unicode decomposition does not reduce to ASCII.
fn fold_special(c: char) -> Option<&'static str> {
    Some(match c {
        'ß' => "ss",
        'æ' | 'Æ' => "ae",
        'œ' | 'Œ' => "oe",
        'ø' | 'Ø' => "o",
        'đ' | 'Đ' | 'ð' | 'Ð' => "d",
        'ł' | 'Ł' => "l",
        'þ' | 'Þ' => "th",
        'ı' => "i",
        _ => return None,
    })
}

/// Turns a tag name into its slug: the ASCII key used in filters and URLs.
///
/// Accents are folded (`Café Noir` → `cafe-noir`); spaces, underscores and
/// punctuation become single hyphens; apostrophes are dropped. Anything that
/// remains outside a-z and 0-9 (such as a non-Latin script) is rejected
/// rather than silently removed, so two different names can't collapse to
/// one slug and lookalike letters can't impersonate a tag.
pub fn tag_slug(input: &str) -> Result<String, ValidationError> {
    use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

    let reject = || ValidationError::TagSlug(input.to_owned());
    let mut folded = String::with_capacity(input.len());
    for c in input.nfkd() {
        if is_combining_mark(c) {
            continue;
        }
        if let Some(s) = fold_special(c) {
            folded.push_str(s);
        } else {
            folded.push(c);
        }
    }

    let mut slug = String::with_capacity(folded.len());
    let mut pending_hyphen = false;
    for c in folded.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_hyphen && !slug.is_empty() {
                slug.push('-');
            }
            pending_hyphen = false;
            slug.push(c.to_ascii_lowercase());
        } else if matches!(c, '\'' | '\u{2019}') {
            // Apostrophes join words: "Writer's" -> "writers".
        } else if c.is_whitespace() || c.is_ascii_punctuation() {
            pending_hyphen = true;
        } else {
            return Err(reject());
        }
    }
    if slug.is_empty() || slug.chars().count() > TAG_SLUG_MAX_CHARS {
        return Err(reject());
    }
    Ok(slug)
}

/// A tag reference written as `namespace:value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagSpec {
    pub namespace: String,
    pub slug: String,
    /// The value as written, used as the display name if the tag is new.
    pub display: String,
}

pub fn tag_spec(input: &str) -> Result<TagSpec, ValidationError> {
    let (ns, value) = input
        .split_once(':')
        .ok_or_else(|| ValidationError::TagSpec(input.to_owned()))?;
    Ok(TagSpec {
        namespace: namespace(ns)?,
        slug: tag_slug(value)?,
        display: tag_display(value)?,
    })
}

/// Parses a prompt's tag list, dropping duplicates.
pub fn tag_specs(inputs: &[String]) -> Result<Vec<TagSpec>, ValidationError> {
    let mut specs: Vec<TagSpec> = Vec::with_capacity(inputs.len());
    for input in inputs {
        let spec = tag_spec(input)?;
        if !specs
            .iter()
            .any(|s| s.namespace == spec.namespace && s.slug == spec.slug)
        {
            specs.push(spec);
        }
    }
    if specs.len() > PROMPT_MAX_TAGS {
        return Err(ValidationError::TooManyTags);
    }
    Ok(specs)
}

/// Parses a comma-separated filter list of `namespace:slug` keys.
pub fn tag_filter(input: Option<&str>) -> Result<Vec<(String, String)>, ValidationError> {
    let Some(raw) = input else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let spec = tag_spec(part)?;
        let key = (spec.namespace, spec.slug);
        if !out.contains(&key) {
            out.push(key);
        }
    }
    if out.len() > PROMPT_MAX_TAGS * 2 {
        return Err(ValidationError::TooManyTags);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_slugs() {
        assert_eq!(tag_slug("Slow burn").unwrap(), "slow-burn");
        assert_eq!(tag_slug("  Café  Noir!! ").unwrap(), "cafe-noir");
        assert_eq!(
            tag_slug("Sci_Fi / Space-Opera").unwrap(),
            "sci-fi-space-opera"
        );
        assert_eq!(tag_slug("Writer's block").unwrap(), "writers-block");
        assert_eq!(tag_slug("Straße").unwrap(), "strasse");
        assert_eq!(tag_slug("Łódź").unwrap(), "lodz");
        assert_eq!(tag_slug("Ærøskøbing").unwrap(), "aeroskobing");
        assert_eq!(tag_slug("ﬁre").unwrap(), "fire"); // ligature
        assert_eq!(tag_slug("ＦＵＬＬ　ｗｉｄｔｈ").unwrap(), "full-width");
        assert_eq!(tag_slug("1920s").unwrap(), "1920s");

        // Non-Latin scripts and lookalike letters are rejected, not dropped.
        assert!(tag_slug("怪談").is_err());
        assert!(tag_slug("\u{0430}pple").is_err()); // Cyrillic а
        assert!(tag_slug("horror 🎃").is_err());
        assert!(tag_slug("!!!").is_err());
        assert!(tag_slug("").is_err());
        assert!(tag_slug(&"a".repeat(TAG_SLUG_MAX_CHARS + 1)).is_err());
    }

    #[test]
    fn namespaces_and_specs() {
        assert_eq!(namespace("Content-Warning").unwrap(), "content-warning");
        assert!(namespace("-cw").is_err());
        assert!(namespace("c w").is_err());
        assert!(namespace("cw--x").is_err());

        let spec = tag_spec("genre:Slow Burn").unwrap();
        assert_eq!(spec.namespace, "genre");
        assert_eq!(spec.slug, "slow-burn");
        assert_eq!(spec.display, "Slow Burn");
        assert!(tag_spec("no-colon").is_err());

        let many: Vec<String> = (0..20).map(|i| format!("theme:t{i}")).collect();
        assert_eq!(tag_specs(&many), Err(ValidationError::TooManyTags));
        let dupes = vec!["genre:Horror".to_owned(), "genre:horror".to_owned()];
        assert_eq!(tag_specs(&dupes).unwrap().len(), 1);

        assert_eq!(
            tag_filter(Some("genre:horror, content-warning:gore,,")).unwrap(),
            vec![
                ("genre".to_owned(), "horror".to_owned()),
                ("content-warning".to_owned(), "gore".to_owned())
            ]
        );
        assert!(tag_filter(None).unwrap().is_empty());
    }

    #[test]
    fn prompt_fields() {
        assert_eq!(prompt_title("  The ledger ").unwrap(), "The ledger");
        assert!(prompt_title("").is_err());
        assert!(prompt_title("two\nlines").is_err());
        assert_eq!(prompt_body("a\r\nb").unwrap(), "a\nb");
        assert!(prompt_body("   ").is_err());
        assert!(prompt_body(&"x".repeat(PROMPT_BODY_MAX_CHARS + 1)).is_err());
    }

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
