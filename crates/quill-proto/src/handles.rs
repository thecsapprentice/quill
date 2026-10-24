//! Reserved persona handles.
//!
//! The list itself is the server's: administrators edit it in Server
//! settings or load it from a file at startup, and it starts out as
//! [`DEFAULT_WORDS`] and [`DEFAULT_EXACT`]. This module decides how entries
//! match, so the server and clients agree.
//!
//! An entry is either
//! - a **word**, which reserves any handle that is the word, has it as a
//!   `_`-separated part (`site_admin`), or spells it with underscores removed
//!   (`ad_min`); or
//! - an **exact** handle, which reserves only that handle.
//!
//! Handles are matched with digits also read as the letters they resemble
//! (`adm1n`). Handles are lowercase `[a-z0-9_]` already (see
//! [`crate::validate::username`]), so case variants are covered.

use serde::{Deserialize, Serialize};

use crate::validate::ValidationError;

crate::board::string_enum! {
    /// How a reserved entry matches.
    ReservedKind {
        /// The whole handle, any `_`-separated part, or the handle with
        /// underscores removed.
        Word => "word",
        /// Only the whole handle.
        Exact => "exact",
    }
}

crate::board::string_enum! {
    /// Where a reserved entry came from.
    ReservedSource {
        /// Quill's starting list; administrators may remove these.
        Default => "default",
        /// Added by an administrator in Server settings or the API.
        Admin => "admin",
        /// Loaded from the file named by `QUILL_RESERVED_HANDLES_FILE`;
        /// changed only by editing that file.
        File => "file",
        /// The server's own name, always reserved; not stored.
        ServerName => "server-name",
    }
}

/// `GET /admin/reserved-handles` query.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
    feature = "openapi",
    derive(utoipa::IntoParams),
    into_params(parameter_in = Query)
)]
#[serde(rename_all = "camelCase")]
pub struct ReservedQuery {
    /// Text the pattern contains.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ReservedKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ReservedSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `GET /admin/reserved-handles` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReservedList {
    pub entries: Vec<ReservedHandle>,
    /// Entries matching the query, including any not returned.
    pub total: u64,
    /// The server's own name, reserved like a word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    /// Whether a reserved-handles file was loaded at startup.
    pub file_loaded: bool,
}

/// `POST /admin/reserved-handles`: reserve several at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AddReserved {
    pub kind: ReservedKind,
    pub patterns: Vec<String>,
}

/// Response to `POST /admin/reserved-handles`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct AddedReserved {
    /// Entries newly reserved; ones already present aren't counted.
    pub added: u64,
}

/// `GET /admin/reserved-handles/check?handle=…`: would a writer be refused?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReservedCheck {
    /// The handle as it would be stored.
    pub handle: String,
    /// The entry that reserves it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_by: Option<ReservedHandle>,
}

/// One reserved entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct ReservedHandle {
    pub pattern: String,
    pub kind: ReservedKind,
    pub source: ReservedSource,
}

/// Starting staff words.
pub const DEFAULT_WORDS: &[&str] = &[
    "admin",
    "admins",
    "administrator",
    "administrators",
    "administration",
    "moderator",
    "moderators",
    "moderation",
    "mod",
    "mods",
    "staff",
    "support",
    "official",
    "sysadmin",
    "sysop",
    "root",
    "superuser",
    "quill",
    "security",
    "abuse",
    "operator",
    "owner",
];

/// Starting exact names: ones that could pass for the system or a page.
pub const DEFAULT_EXACT: &[&str] = &[
    "all",
    "everyone",
    "here",
    "nobody",
    "null",
    "none",
    "undefined",
    "anonymous",
    "anon",
    "system",
    "server",
    "instance",
    "api",
    "www",
    "mail",
    "email",
    "postmaster",
    "webmaster",
    "hostmaster",
    "noreply",
    "no_reply",
    "help",
    "info",
    "contact",
    "legal",
    "privacy",
    "terms",
    "report",
    "reports",
    "settings",
    "login",
    "logout",
    "register",
    "signup",
    "new",
    "me",
    "inbox",
    "outbox",
    "followers",
    "following",
    "actor",
    "users",
    "tags",
    "prompts",
    "static",
];

/// Readings of `s`: as written, and with digits taken as the letters they
/// resemble (`1` could be `i` or `l`, so both).
fn readings(s: &str) -> Vec<String> {
    let read = |one: char| -> String {
        s.chars()
            .map(|c| match c {
                '0' => 'o',
                '1' => one,
                '3' => 'e',
                '4' => 'a',
                '5' => 's',
                '7' => 't',
                '8' => 'b',
                other => other,
            })
            .collect()
    };
    let mut out = vec![s.to_owned(), read('i'), read('l')];
    out.dedup();
    out
}

/// What a handle is checked against: `(words, exact)`. A word entry
/// reserves the handle if it equals any string in `words`; an exact entry,
/// any string in `exact`.
pub fn candidates(handle: &str) -> (Vec<String>, Vec<String>) {
    let mut words = Vec::new();
    let mut exact = Vec::new();
    for h in readings(handle) {
        words.extend(h.split('_').filter(|p| !p.is_empty()).map(str::to_owned));
        words.push(h.chars().filter(|c| *c != '_').collect());
        exact.push(h);
    }
    words.sort();
    words.dedup();
    exact.sort();
    exact.dedup();
    (words, exact)
}

/// Normalizes an entry as typed: lowercase `[a-z0-9_]`, 1 to 64 characters.
/// Words are stored without underscores, since that's how they match.
pub fn normalize_entry(kind: ReservedKind, raw: &str) -> Result<String, ValidationError> {
    let lowered = raw.trim().to_ascii_lowercase();
    let ok = (1..=64).contains(&lowered.len())
        && lowered
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !ok {
        return Err(ValidationError::ReservedEntry(raw.trim().to_owned()));
    }
    Ok(match kind {
        ReservedKind::Word => lowered.chars().filter(|c| *c != '_').collect(),
        ReservedKind::Exact => lowered,
    })
    .and_then(|s: String| {
        if s.is_empty() {
            Err(ValidationError::ReservedEntry(raw.trim().to_owned()))
        } else {
            Ok(s)
        }
    })
}

/// A bad line in a reserved-handles file: its 1-based number and why.
pub type FileError = (usize, String);

/// Parses a reserved-handles file: one entry per line, a word unless it
/// starts with `=` (exact). Blank lines and `#` comments are skipped. Errors
/// carry 1-based line numbers.
pub fn parse_file(text: &str) -> Result<Vec<(ReservedKind, String)>, Vec<FileError>> {
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let (kind, raw) = match line.strip_prefix('=') {
            Some(rest) => (ReservedKind::Exact, rest),
            None => (ReservedKind::Word, line),
        };
        match normalize_entry(kind, raw) {
            Ok(p) => entries.push((kind, p)),
            Err(e) => errors.push((i + 1, e.to_string())),
        }
    }
    if errors.is_empty() {
        entries.sort();
        entries.dedup();
        Ok(entries)
    } else {
        Err(errors)
    }
}

/// The server's own name from its domain, as a handle would spell it:
/// `writers` for `writers.example`, `mysite` for `my-site.example`. Always
/// reserved like a word.
pub fn server_name(domain: &str) -> Option<String> {
    let label: String = domain
        .split('.')
        .next()?
        .to_ascii_lowercase()
        .chars()
        .filter(|c| *c != '-')
        .collect();
    (!label.is_empty()
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
    .then_some(label)
}

/// Whether `handle` is reserved by an in-memory list of `(kind, pattern)`.
/// The server checks its table in the database the same way.
pub fn is_reserved(handle: &str, entries: &[(ReservedKind, &str)]) -> bool {
    let (words, exact) = candidates(handle);
    entries.iter().any(|(kind, pattern)| match kind {
        ReservedKind::Word => words.iter().any(|w| w == pattern),
        ReservedKind::Exact => exact.iter().any(|e| e == pattern),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults_and(server: &str) -> Vec<(ReservedKind, &str)> {
        DEFAULT_WORDS
            .iter()
            .map(|w| (ReservedKind::Word, *w))
            .chain(DEFAULT_EXACT.iter().map(|e| (ReservedKind::Exact, *e)))
            .chain([(ReservedKind::Word, server)])
            .collect()
    }

    #[test]
    fn default_rules() {
        let list = defaults_and("lantern");
        for h in [
            "admin",
            "site_admin",
            "admin_bob",
            "ad_min",
            "adm1n",
            "m0derator",
            "supp0rt",
            "the_quill",
            "r00t",
            "everyone",
            "no_reply",
            "lantern",
            "lantern_team",
            "lant_ern",
            "1antern",
            "mod_squad_fan2",
        ] {
            assert!(is_reserved(h, &list), "{h} should be reserved");
        }
        for h in [
            "ink_fox",
            "badminton",
            "moderate_reader",
            "allison",
            "helpful_otter",
            "lanterns",
            "quilled",
            "all_in",
        ] {
            assert!(!is_reserved(h, &list), "{h} should be allowed");
        }
    }

    #[test]
    fn entries_and_files() {
        assert_eq!(
            normalize_entry(ReservedKind::Word, " No_Reply ").unwrap(),
            "noreply"
        );
        assert_eq!(
            normalize_entry(ReservedKind::Exact, "No_Reply").unwrap(),
            "no_reply"
        );
        for bad in ["", "two words", "café", "a-b", "___", &"x".repeat(65)] {
            let kind = if bad == "___" {
                ReservedKind::Word
            } else {
                ReservedKind::Exact
            };
            assert!(normalize_entry(kind, bad).is_err(), "{bad:?}");
        }
        let parsed = parse_file("# brands\nAcme\n=acme_help  # exact\n\nacme\n").unwrap();
        assert_eq!(
            parsed,
            vec![
                (ReservedKind::Word, "acme".to_owned()),
                (ReservedKind::Exact, "acme_help".to_owned())
            ]
        );
        let errors = parse_file("fine\nnot fine\n=\n").unwrap_err();
        assert_eq!(
            errors.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn server_names() {
        assert_eq!(server_name("Writers.Example").as_deref(), Some("writers"));
        assert_eq!(server_name("localhost").as_deref(), Some("localhost"));
        assert_eq!(server_name("my-site.example").as_deref(), Some("mysite"));
        assert!(is_reserved("my_site", &[(ReservedKind::Word, "mysite")]));
        assert_eq!(server_name("bad_name!.example"), None);
        assert_eq!(server_name(""), None);
    }
}
