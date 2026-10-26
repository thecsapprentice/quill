//! The engine-independent `text` API that every engine registers.
//!
//! Plain Rust functions over strings and lists, so each engine only needs a
//! thin adapter. Patterns and word lists are compiled on first use and
//! cached by their source, so scripts can pass them inline without paying
//! for a rebuild on every post.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use aho_corasick::{AhoCorasick, MatchKind};
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;
use unicode_security::MixedScript;
use unicode_security::confusable_detection::skeleton as uts39_skeleton;
use unicode_segmentation::UnicodeSegmentation;

/// Compiled-pattern size cap, so one admin pattern can't take a lot of memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

thread_local! {
    static REGEX_CACHE: RefCell<HashMap<String, Rc<Regex>>> = RefCell::new(HashMap::new());
    static WORDS_CACHE: RefCell<HashMap<Vec<String>, Rc<WordList>>> = RefCell::new(HashMap::new());
}

/// A word list folded into its comparable form, as one automaton.
struct WordList {
    automaton: AhoCorasick,
    originals: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub url: String,
    pub host: String,
}

/// Case-insensitive, Unicode-aware regex, compiled once per pattern.
fn regex(pattern: &str) -> Result<Rc<Regex>, String> {
    REGEX_CACHE.with(|cache| {
        if let Some(re) = cache.borrow().get(pattern) {
            return Ok(re.clone());
        }
        let re = RegexBuilder::new(pattern)
            .case_insensitive(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .build()
            .map_err(|e| format!("invalid pattern {pattern:?}: {e}"))?;
        let re = Rc::new(re);
        cache.borrow_mut().insert(pattern.to_owned(), re.clone());
        Ok(re)
    })
}

pub fn matches(pattern: &str, text: &str) -> Result<bool, String> {
    Ok(regex(pattern)?.is_match(text))
}

pub fn find_all(pattern: &str, text: &str) -> Result<Vec<String>, String> {
    Ok(regex(pattern)?.find_iter(text).map(|m| m.as_str().to_owned()).collect())
}

/// Named groups of the first match, or `None` if nothing matched.
pub fn captures(pattern: &str, text: &str) -> Result<Option<HashMap<String, String>>, String> {
    let re = regex(pattern)?;
    Ok(re.captures(text).map(|caps| {
        re.capture_names()
            .flatten()
            .filter_map(|name| caps.name(name).map(|m| (name.to_owned(), m.as_str().to_owned())))
            .collect()
    }))
}

pub fn replace(pattern: &str, text: &str, with: &str) -> Result<String, String> {
    Ok(regex(pattern)?.replace_all(text, with).into_owned())
}

/// NFKC, lower case, no accents and no invisible characters.
pub fn normalize(text: &str) -> String {
    text.nfkd()
        .filter(|c| !is_combining_mark(*c) && !is_invisible(*c))
        .nfkc()
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_invisible(c: char) -> bool {
    matches!(c, '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
}

/// Lookalike characters mapped to one prototype (Unicode TS #39), so
/// `pаypal` with a Cyrillic `а` reads as `paypal`.
pub fn skeleton(text: &str) -> String {
    let normalized = normalize(text);
    uts39_skeleton(&normalized).flat_map(char::to_lowercase).collect()
}

/// Repeated letters collapsed ("spaaam" → "spam") and runs of three or more
/// single letters joined ("s p a m", "s.p.a.m" → "spam").
pub fn squash(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut last = None;
    for c in text.chars() {
        if Some(c) != last || !c.is_alphanumeric() {
            collapsed.push(c);
        }
        last = Some(c);
    }

    let tokens: Vec<&str> = collapsed.split_word_bounds().collect();
    let mut out = String::with_capacity(collapsed.len());
    let mut i = 0;
    while i < tokens.len() {
        // Find a run of single letters with only separators between them.
        let mut j = i;
        let mut letters = String::new();
        while j < tokens.len() && is_single_letter(tokens[j]) {
            letters.push_str(tokens[j]);
            j += 1;
            if j + 1 < tokens.len() && is_separator(tokens[j]) && is_single_letter(tokens[j + 1]) {
                j += 1;
            }
        }
        if letters.chars().count() >= 3 {
            out.push_str(&letters);
            i = j;
        } else {
            out.push_str(tokens[i]);
            i += 1;
        }
    }
    out
}

fn is_single_letter(token: &str) -> bool {
    let mut chars = token.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c.is_alphanumeric())
}

fn is_separator(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|c| c.is_whitespace() || matches!(c, '.' | '-' | '_' | '*'))
}

/// The form both sides are compared in by `find_words`. Squashing comes
/// first because skeletons can turn one letter into two (`m` → `rn`).
pub fn fold(text: &str) -> String {
    skeleton(&squash(&normalize(text)))
}

/// Which entries of `list` appear in `text` as whole words, after folding
/// both, so spacing, repeats, accents and lookalikes don't hide them.
pub fn find_words(list: &[String], text: &str) -> Result<Vec<String>, String> {
    let words = word_list(list)?;
    let folded = fold(text);
    let mut found: Vec<String> = Vec::new();
    for m in words.automaton.find_iter(&folded) {
        let before = folded[..m.start()].chars().next_back();
        let after = folded[m.end()..].chars().next();
        let bounded = !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric);
        let original = &words.originals[m.pattern().as_usize()];
        if bounded && !found.contains(original) {
            found.push(original.clone());
        }
    }
    Ok(found)
}

fn word_list(list: &[String]) -> Result<Rc<WordList>, String> {
    WORDS_CACHE.with(|cache| {
        if let Some(words) = cache.borrow().get(list) {
            return Ok(words.clone());
        }
        let folded: Vec<String> = list.iter().map(|w| fold(w)).collect();
        let automaton = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&folded)
            .map_err(|e| format!("word list: {e}"))?;
        let words = Rc::new(WordList { automaton, originals: list.to_vec() });
        cache.borrow_mut().insert(list.to_vec(), words.clone());
        Ok(words)
    })
}

/// Links with and without a scheme ("example.com/x" counts), with the host
/// lower-cased and any leading "www." removed.
pub fn links(text: &str) -> Vec<Link> {
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]).url_must_have_scheme(false);
    finder
        .links(text)
        .filter_map(|link| {
            let raw = link.as_str();
            let with_scheme = if raw.contains("://") { raw.to_owned() } else { format!("https://{raw}") };
            let host = url::Url::parse(&with_scheme).ok()?.host_str()?.to_lowercase();
            let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
            Some(Link { url: raw.to_owned(), host })
        })
        .collect()
}

pub fn words(text: &str) -> Vec<String> {
    text.unicode_words().map(str::to_owned).collect()
}

/// Words that mix writing systems, like Latin with one Cyrillic letter.
pub fn mixed_script_words(text: &str) -> Vec<String> {
    text.unicode_words().filter(|w| !w.is_single_script()).map(str::to_owned).collect()
}

/// 0.0 (nothing alike) to 1.0 (the same), after normalizing both.
pub fn similarity(a: &str, b: &str) -> f64 {
    strsim::normalized_levenshtein(&normalize(a), &normalize(b))
}

/// Share of letters that are capitals, 0.0 when there are no letters.
pub fn caps_ratio(text: &str) -> f64 {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    if letters == 0 {
        return 0.0;
    }
    text.chars().filter(|c| c.is_uppercase()).count() as f64 / letters as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn finds_words_through_evasions() {
        let banned = list(&["spam", "free crypto", "paypal"]);
        assert_eq!(find_words(&banned, "Buy S P A M now").unwrap(), ["spam"]);
        assert_eq!(find_words(&banned, "spaaaam!").unwrap(), ["spam"]);
        assert_eq!(find_words(&banned, "FREE crypto here").unwrap(), ["free crypto"]);
        assert_eq!(find_words(&banned, "log in to p\u{0430}ypal").unwrap(), ["paypal"]);
        assert!(find_words(&banned, "a spammer's lament").unwrap().is_empty());
    }

    #[test]
    fn finds_links_without_schemes() {
        let found = links("see https://www.Example.com/x and evil.biz/win");
        let hosts: Vec<_> = found.iter().map(|l| l.host.as_str()).collect();
        assert_eq!(hosts, ["example.com", "evil.biz"]);
    }

    #[test]
    fn flags_mixed_scripts() {
        assert_eq!(mixed_script_words("hello p\u{0430}ypal"), ["p\u{0430}ypal"]);
        assert!(mixed_script_words("Привет hello").is_empty());
    }
}
