//! Reserved persona handles: names that could pass for the server's staff,
//! the server itself, or a system page. Writers can't take them; only the
//! server's `grant-handle` command can.
//!
//! Handles are already lowercase ASCII `[a-z0-9_]` (see
//! [`crate::validate::username`]), so case variants are covered by the time
//! a handle reaches here.

/// Words that suggest staff. Reserved as a whole handle, as any
/// `_`-separated part (`site_admin`), and with underscores removed
/// (`ad_min`).
const STAFF_WORDS: &[&str] = &[
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

/// Names reserved only as the whole handle: they could pass for the system
/// or a page of the site.
const EXACT_NAMES: &[&str] = &[
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

/// Readings of `s` with digits taken as the letters they resemble. `1`
/// could be `i` or `l`, so both are tried.
fn readings(s: &str) -> [String; 2] {
    let read = |one: char| {
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
    [read('i'), read('l')]
}

/// The server's own name from its domain, as a handle would spell it:
/// `writers` for `writers.example`, `mysite` for `my-site.example`.
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

/// Whether `handle` (already normalized) is reserved. `server_names` adds
/// names reserved like staff words, such as the server's own name.
pub fn is_reserved(handle: &str, server_names: &[&str]) -> bool {
    let staff = |s: &str| STAFF_WORDS.contains(&s) || server_names.contains(&s);
    readings(handle).iter().any(|h| {
        let joined: String = h.chars().filter(|c| *c != '_').collect();
        EXACT_NAMES.contains(&h.as_str()) || staff(&joined) || h.split('_').any(staff)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_names() {
        let server = ["lantern"];
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
        ] {
            assert!(is_reserved(h, &server), "{h} should be reserved");
        }
        for h in [
            "ink_fox",
            "badminton",
            "moderate_reader",
            "allison",
            "helpful_otter",
            "lanterns",
            "quilled",
            "mod_squad_fan2",
        ] {
            let expected = h == "mod_squad_fan2";
            assert_eq!(is_reserved(h, &server), expected, "{h}");
        }
    }

    #[test]
    fn server_names() {
        assert_eq!(server_name("Writers.Example").as_deref(), Some("writers"));
        assert_eq!(server_name("localhost").as_deref(), Some("localhost"));
        assert_eq!(server_name("my-site.example").as_deref(), Some("mysite"));
        assert!(is_reserved("my_site", &["mysite"]));
        assert_eq!(server_name("bad_name!.example"), None);
        assert_eq!(server_name(""), None);
    }
}
