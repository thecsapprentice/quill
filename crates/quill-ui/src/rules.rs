//! The rules for what writers type, said as they type rather than after.
//! Each check returns the line shown under the field: the rule while the
//! field is empty or fine, and what's wrong once it isn't.

use quill_proto::{
    limits::{DISPLAY_NAME_MAX_CHARS, PASSWORD_MIN_CHARS, USERNAME_MAX_CHARS},
    validate,
};

use crate::backend::PASSPHRASE_MIN_CHARS;

/// The line under a field, and whether it says something is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub hint: String,
    pub invalid: bool,
}

impl Check {
    fn fine(hint: impl Into<String>) -> Self {
        Self {
            hint: hint.into(),
            invalid: false,
        }
    }

    fn wrong(hint: impl Into<String>) -> Self {
        Self {
            hint: hint.into(),
            invalid: true,
        }
    }
}

/// A login name or a persona handle: the same rule on the server.
fn name(input: &str, rule: &str, shown: impl Fn(&str) -> String) -> Check {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Check::fine(rule);
    }
    if let Ok(normal) = validate::username(trimmed) {
        return Check::fine(shown(&normal));
    }
    let count = trimmed.chars().count();
    if count > USERNAME_MAX_CHARS {
        return Check::wrong(format!(
            "That's {count} characters; the most is {USERNAME_MAX_CHARS}."
        ));
    }
    if trimmed.contains(char::is_whitespace) {
        return Check::wrong("No spaces. Use _ to join words.");
    }
    Check::wrong("Only the letters a to z, numbers and _ work here.")
}

pub fn login_name(input: &str) -> Check {
    name(
        input,
        &format!("Up to {USERNAME_MAX_CHARS} letters a to z, numbers and _. Only you see it."),
        |n| format!("You'll sign in as {n}. Only you see it."),
    )
}

pub fn handle(input: &str) -> Check {
    name(
        input,
        &format!("Up to {USERNAME_MAX_CHARS} letters a to z, numbers and _."),
        |n| format!("Others will see you as @{n}."),
    )
}

/// How many more characters a secret needs.
fn length(input: &str, min: usize) -> Check {
    let have = input.chars().count();
    match have {
        0 => Check::fine(format!("At least {min} characters.")),
        n if n < min => Check::wrong(format!(
            "{} more character{} to go.",
            min - n,
            if min - n == 1 { "" } else { "s" }
        )),
        _ => Check::fine("Long enough."),
    }
}

pub fn password(input: &str) -> Check {
    length(input, PASSWORD_MIN_CHARS)
}

pub fn passphrase(input: &str) -> Check {
    length(input, PASSPHRASE_MIN_CHARS)
}

pub fn display_name(input: &str) -> Check {
    match validate::display_name(Some(input)) {
        Ok(_) => Check::fine("Optional. Shown beside your handle; spaces and any script are fine."),
        Err(_) => Check::wrong(format!(
            "Keep it to one line of at most {DISPLAY_NAME_MAX_CHARS} characters."
        )),
    }
}

/// Handles near one that's taken, for the writer to pick from. The server
/// may have some of these too; trying one says so.
pub fn suggestions(taken: &str) -> Vec<String> {
    let Ok(base) = validate::username(taken) else {
        return Vec::new();
    };
    let fit = |s: String| -> Option<String> { validate::username(&s).ok().filter(|v| *v != base) };
    let short: String = base.chars().take(USERNAME_MAX_CHARS - 7).collect();
    [
        format!("{short}_writes"),
        format!("the_{short}"),
        format!("{short}_ink"),
        format!("{short}{}", short.len() % 7 + 2),
    ]
    .into_iter()
    .filter_map(fit)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_say_what_is_wrong_as_you_type() {
        assert!(!login_name("").invalid);
        assert!(
            login_name("Ink").hint.contains("ink"),
            "capitals are lowered"
        );
        assert!(login_name("ink fox").hint.contains("No spaces"));
        assert!(login_name("ink-fox").invalid);
        assert!(login_name(&"a".repeat(31)).hint.contains("31 characters"));
        assert_eq!(handle("ink_fox").hint, "Others will see you as @ink_fox.");
    }

    #[test]
    fn secrets_count_down_to_long_enough() {
        assert!(!password("").invalid);
        assert_eq!(password("eleven char").hint, "1 more character to go.");
        assert!(password("eleven char").invalid);
        assert!(!password("twelve chars").invalid);
        assert!(passphrase("short").invalid);
    }

    #[test]
    fn display_names_are_one_line() {
        assert!(!display_name("Ink Fox ✒️").invalid);
        assert!(display_name("two\nlines").invalid);
    }

    #[test]
    fn suggestions_are_valid_and_different() {
        let s = suggestions("ink_fox");
        assert!(s.len() >= 3);
        for h in &s {
            assert!(validate::username(h).is_ok(), "{h}");
            assert_ne!(h, "ink_fox");
        }
        // Long handles still give valid suggestions.
        for h in suggestions(&"a".repeat(30)) {
            assert!(validate::username(&h).is_ok(), "{h}");
        }
        assert!(suggestions("not valid").is_empty());
    }
}
