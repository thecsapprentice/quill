//! Error codes (#76): every problem the server or a client can show has
//! one, written `Q-3009`. The first two digits name the area and the last
//! two the case. In each area, 01–49 are the server's refusals and 50–99
//! are what a client decides itself. `docs/error-codes.md` is generated
//! from this registry; codes never change meaning, and retired ones are
//! never reused (`RETIRED`).

use std::fmt;

use serde::{Deserialize, Serialize};

/// One error code. Serialized as its number; shown as `Q-3009`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ErrorCode(pub u16);

impl ErrorCode {
    /// The area's number: `30` for `Q-3009`.
    pub fn area(self) -> u16 {
        self.0 / 100
    }

    /// Whether the server decided it (01–49), rather than a client (50–99).
    pub fn from_server(self) -> bool {
        self.0 % 100 < 50
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Q-{:04}", self.0)
    }
}

/// A registered code: its constant's name and what it means.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub code: ErrorCode,
    pub name: &'static str,
    pub meaning: &'static str,
}

/// Declares codes as constants and lists them in `TABLE`. The doc comment
/// on each is its meaning in `docs/error-codes.md`.
macro_rules! registry {
    ($( $(#[doc = $doc:literal])+ $name:ident = $n:literal, )*) => {
        $( $(#[doc = $doc])+ pub const $name: super::ErrorCode = super::ErrorCode($n); )*
        pub(super) const TABLE: &[super::Entry] = &[
            $( super::Entry { code: $name, name: stringify!($name), meaning: concat!($($doc),+) }, )*
        ];
    };
}

mod client;
mod server;

pub use client::*;
pub use server::*;

/// The areas, by their two-digit number.
pub const AREAS: &[(u16, &str)] = &[
    (10, "Connection and the server"),
    (11, "Sign-in and logins"),
    (12, "Server rules"),
    (13, "This computer's store and keychain"),
    (14, "Personas and handles"),
    (15, "Identity keys, pins and KeyPackages"),
    (16, "Devices and linking"),
    (20, "Prompts"),
    (21, "Tags and tag types"),
    (22, "Reports, notices and automated moderation"),
    (23, "Announcements and comments"),
    (24, "Administration and server settings"),
    (30, "Requests"),
    (31, "Sessions and groups"),
    (32, "Messages and channels"),
    (33, "Attachments and storage"),
    (34, "Backup and recovery"),
    (35, "History and sync"),
    (90, "Input checks shared by every area"),
    (99, "Not yet given a code of its own"),
];

/// Codes once used and now retired: never reused.
pub const RETIRED: &[u16] = &[];

/// Every registered code, in order.
pub fn all() -> Vec<Entry> {
    let mut v: Vec<Entry> = server::TABLE.iter().chain(client::TABLE).copied().collect();
    v.sort_by_key(|e| e.code);
    v
}

/// What a code means, if it's registered.
pub fn meaning(code: ErrorCode) -> Option<&'static str> {
    all()
        .into_iter()
        .find(|e| e.code == code)
        .map(|e| e.meaning.trim())
}

/// `docs/error-codes.md`, generated.
pub fn document() -> String {
    let mut out = String::from(
        "# Error codes\n\n\
         Generated from `quill_proto::codes`; don't edit by hand \
         (`QUILL_WRITE_CODES=1 cargo test -p quill-proto codes`).\n\n\
         Every problem Quill can show has a code, `Q-AACC`: `AA` is the area and \
         `CC` the case. In each area, 01–49 are refusals by the server and \
         50–99 are decided by the client. The desktop app shows codes in \
         advanced mode; the CLI always prints them. A code never changes \
         meaning, and a retired code is never reused.\n",
    );
    let entries = all();
    for (area, name) in AREAS {
        let mine: Vec<&Entry> = entries.iter().filter(|e| e.code.area() == *area).collect();
        if mine.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "\n## Q-{area}xx: {name}\n\n| Code | Name | Meaning |\n| --- | --- | --- |\n"
        ));
        for e in mine {
            out.push_str(&format!(
                "| {} | `{}` | {} |\n",
                e.code,
                e.name,
                e.meaning.trim()
            ));
        }
    }
    if !RETIRED.is_empty() {
        out.push_str("\n## Retired\n\n");
        for r in RETIRED {
            out.push_str(&format!("- {}\n", ErrorCode(*r)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::collections::HashSet;

    use super::*;

    #[test]
    fn codes_are_unique_in_a_known_area_and_on_the_right_side() {
        let mut seen = HashSet::new();
        for e in all() {
            assert!(seen.insert(e.code), "{} is used twice", e.code);
            assert!(
                AREAS.iter().any(|(a, _)| *a == e.code.area()),
                "{} has no area",
                e.code
            );
            assert!(
                (1000..10_000).contains(&e.code.0),
                "{} isn't four digits",
                e.code
            );
            assert!(!RETIRED.contains(&e.code.0), "{} was retired", e.code);
            assert!(!e.meaning.trim().is_empty(), "{} has no meaning", e.name);
        }
        for e in server::TABLE {
            assert!(
                e.code.from_server(),
                "{} ({}) is in the client's range",
                e.code,
                e.name
            );
        }
        for e in client::TABLE {
            assert!(
                !e.code.from_server() || e.code.area() == 90,
                "{} ({}) is in the server's range",
                e.code,
                e.name
            );
        }
    }

    #[test]
    fn codes_read_as_q_and_four_digits() {
        assert_eq!(ErrorCode(3009).to_string(), "Q-3009");
        assert_eq!(serde_json::to_string(&ErrorCode(3009)).unwrap(), "3009");
    }

    #[test]
    fn the_committed_error_codes_file_is_what_the_registry_generates() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/error-codes.md");
        let text = document();
        if std::env::var_os("QUILL_WRITE_CODES").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{} missing; set QUILL_WRITE_CODES=1", path.display()));
        assert!(
            committed == text,
            "docs/error-codes.md is stale: QUILL_WRITE_CODES=1 cargo test -p quill-proto codes"
        );
    }
}
