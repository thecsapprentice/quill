//! The `text` API moderation scripts use: regular expressions, word lists,
//! normalizing against evasions, structure and measures.
//!
//! Plain Rust with no knowledge of the scripting engine, so it can be tested
//! on its own and the engine only needs thin adapters. Design notes are in
//! the architecture document, "Automated moderation".

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use aho_corasick::{AhoCorasick, MatchKind};
use regex::{Regex, RegexBuilder};
use serde::Serialize;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;
use unicode_script::{Script, UnicodeScript};
use unicode_security::MixedScript;
use unicode_security::confusable_detection::skeleton as uts39_skeleton;
use unicode_segmentation::UnicodeSegmentation;

use crate::markdown;

/// How large one compiled pattern may grow, so one admin pattern can't take
/// a lot of memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// Entries each cache keeps before it starts over.
const CACHE_ENTRIES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TextError(String);

/// One match: its text and where it is, counted in characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Found {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    pub url: String,
    /// Lower case, without a leading `www.`.
    pub host: String,
}

/// Compiled patterns and word lists for one script, so a pattern written
/// inline is compiled once rather than on every post. Bounded: when full it
/// starts over, which only costs recompiling.
#[derive(Debug, Default)]
pub struct TextCache {
    regexes: Mutex<HashMap<String, Arc<Regex>>>,
    words: Mutex<HashMap<Vec<String>, Arc<WordList>>>,
}

fn cached<K, V>(
    map: &Mutex<HashMap<K, Arc<V>>>,
    key: &K,
    make: impl FnOnce() -> Result<V, TextError>,
) -> Result<Arc<V>, TextError>
where
    K: std::hash::Hash + Eq + Clone,
{
    if let Some(hit) = map.lock().unwrap_or_else(PoisonError::into_inner).get(key) {
        return Ok(hit.clone());
    }
    // Built outside the lock: a large word list takes a while.
    let value = Arc::new(make()?);
    let mut map = map.lock().unwrap_or_else(PoisonError::into_inner);
    if map.len() >= CACHE_ENTRIES {
        map.clear();
    }
    map.insert(key.clone(), value.clone());
    Ok(value)
}

impl TextCache {
    /// Case-insensitive and Unicode-aware unless the pattern says otherwise
    /// with `(?-i)`.
    pub fn regex(&self, pattern: &str) -> Result<Arc<Regex>, TextError> {
        cached(&self.regexes, &pattern.to_owned(), || compile(pattern))
    }

    pub fn word_list(&self, words: &[String]) -> Result<Arc<WordList>, TextError> {
        cached(&self.words, &words.to_vec(), || WordList::new(words))
    }
}

/// Compiles a pattern the way scripts get it, so upload can check literal
/// patterns before a script runs.
pub fn compile(pattern: &str) -> Result<Regex, TextError> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .dfa_size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| TextError(format!("invalid pattern {pattern:?}: {e}")))
}

/// Turns byte offsets into character offsets, walking forward once.
struct CharCounter<'a> {
    text: &'a str,
    byte: usize,
    chars: usize,
}

impl<'a> CharCounter<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            byte: 0,
            chars: 0,
        }
    }

    /// `byte` must not be behind the last call's.
    fn at(&mut self, byte: usize) -> usize {
        self.chars += self.text[self.byte..byte].chars().count();
        self.byte = byte;
        self.chars
    }

    fn found(&mut self, m: regex::Match<'_>) -> Found {
        let start = self.at(m.start());
        let end = self.at(m.end());
        Found {
            text: m.as_str().to_owned(),
            start,
            end,
        }
    }
}

pub fn is_match(re: &Regex, text: &str) -> bool {
    re.is_match(text)
}

pub fn find(re: &Regex, text: &str) -> Option<Found> {
    re.find(text).map(|m| CharCounter::new(text).found(m))
}

pub fn find_all(re: &Regex, text: &str) -> Vec<Found> {
    let mut counter = CharCounter::new(text);
    re.find_iter(text).map(|m| counter.found(m)).collect()
}

/// The first match's groups, by name and by number ("0" is the whole
/// match). Groups that didn't take part are left out.
pub fn captures(re: &Regex, text: &str) -> Option<HashMap<String, String>> {
    let caps = re.captures(text)?;
    let mut out = HashMap::new();
    for (i, name) in re.capture_names().enumerate() {
        if let Some(m) = caps.get(i) {
            out.insert(i.to_string(), m.as_str().to_owned());
            if let Some(name) = name {
                out.insert(name.to_owned(), m.as_str().to_owned());
            }
        }
    }
    Some(out)
}

/// Replaces every match; `$1` and `$name` in `with` refer to groups.
pub fn replace(re: &Regex, text: &str, with: &str) -> String {
    re.replace_all(text, with).into_owned()
}

pub fn split(re: &Regex, text: &str) -> Vec<String> {
    re.split(text).map(str::to_owned).collect()
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
    uts39_skeleton(&normalize(text))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Runs of three or more single letters joined ("s p a m", "s.p.a.m" →
/// "spam"), then repeated letters collapsed ("spaaam" → "spam"). Joining
/// comes first so "f r e e" and "free" end up the same ("fre").
pub fn squash(text: &str) -> String {
    let joined = join_letters(text);
    let mut out = String::with_capacity(joined.len());
    let mut last = None;
    for c in joined.chars() {
        if Some(c) != last || !c.is_alphanumeric() {
            out.push(c);
        }
        last = Some(c);
    }
    out
}

fn join_letters(text: &str) -> String {
    // Word segmentation keeps "s.p.a.m" as one word (like "e.g."), so
    // dotted runs are joined here; spaced runs are joined below.
    let tokens: Vec<std::borrow::Cow<'_, str>> = text.split_word_bounds().map(undot).collect();
    let tokens: Vec<&str> = tokens.iter().map(|t| t.as_ref()).collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < tokens.len() {
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

/// "s.p.a.m" → "spam": three or more single letters, each pair split by
/// one separator. Anything else is returned as it was.
fn undot(token: &str) -> std::borrow::Cow<'_, str> {
    let chars: Vec<char> = token.chars().collect();
    let dotted = chars.len() >= 5
        && chars.iter().enumerate().all(|(i, c)| {
            if i % 2 == 0 {
                c.is_alphanumeric()
            } else {
                matches!(c, '.' | '\'' | '_' | '-')
            }
        })
        && chars.len() % 2 == 1;
    if dotted {
        chars.iter().step_by(2).collect::<String>().into()
    } else {
        token.into()
    }
}

fn is_single_letter(token: &str) -> bool {
    let mut chars = token.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c.is_alphanumeric())
}

fn is_separator(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_whitespace() || matches!(c, '.' | '-' | '_' | '*'))
}

/// The form `find_words` compares both sides in. Squashing comes before the
/// skeleton because a skeleton can turn one letter into two (`m` → `rn`).
pub fn fold(text: &str) -> String {
    skeleton(&squash(&normalize(text)))
}

/// A list of words and phrases, folded and built into one automaton, so
/// checking thousands of entries costs one pass over the text.
#[derive(Debug)]
pub struct WordList {
    automaton: AhoCorasick,
    originals: Vec<String>,
}

impl WordList {
    pub fn new(words: &[impl AsRef<str>]) -> Result<Self, TextError> {
        let originals: Vec<String> = words
            .iter()
            .map(|w| w.as_ref().trim().to_owned())
            .filter(|w| !w.is_empty())
            .collect();
        let folded: Vec<String> = originals.iter().map(|w| fold(w)).collect();
        let automaton = AhoCorasick::builder()
            .match_kind(MatchKind::Standard)
            .build(&folded)
            .map_err(|e| TextError(format!("word list: {e}")))?;
        Ok(Self {
            automaton,
            originals,
        })
    }

    /// The entries that appear in `text` as whole words, after folding both,
    /// so spacing, repeats, accents and lookalikes don't hide them. Each
    /// entry is listed once, in list order.
    pub fn find_in(&self, text: &str) -> Vec<String> {
        let folded = fold(text);
        let mut hit = vec![false; self.originals.len()];
        // Overlapping search, so a longer entry failing the word-boundary
        // check can't hide a shorter one at the same place.
        for m in self.automaton.find_overlapping_iter(&folded) {
            let before = folded[..m.start()].chars().next_back();
            let after = folded[m.end()..].chars().next();
            if !before.is_some_and(char::is_alphanumeric)
                && !after.is_some_and(char::is_alphanumeric)
            {
                hit[m.pattern().as_usize()] = true;
            }
        }
        self.originals
            .iter()
            .zip(hit)
            .filter(|(_, h)| *h)
            .map(|(w, _)| w.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.originals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.originals.is_empty()
    }
}

/// Every link in prompt Markdown: written as Markdown links or images, and
/// bare ones in the text, with or without a scheme ("example.com/x"
/// counts). Mail addresses aren't links.
pub fn links(markdown_text: &str) -> Vec<Link> {
    let mut finder = linkify::LinkFinder::new();
    finder
        .kinds(&[linkify::LinkKind::Url])
        .url_must_have_scheme(false);
    let bare = markdown::plain_text(markdown_text);
    let mut out: Vec<Link> = Vec::new();
    let mut seen: Vec<url::Url> = Vec::new();
    let destinations = markdown::link_destinations(markdown_text);
    let found = destinations
        .iter()
        .map(String::as_str)
        .chain(finder.links(&bare).map(|l| l.as_str()));
    for raw in found {
        // The same address written twice (as a Markdown link and bare) is
        // listed once.
        if let Some((link, parsed)) = to_link(raw)
            && !seen.contains(&parsed)
        {
            seen.push(parsed);
            out.push(link);
        }
    }
    out
}

fn to_link(raw: &str) -> Option<(Link, url::Url)> {
    let with_scheme = if raw.contains("://") || raw.starts_with("mailto:") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let url = url::Url::parse(&with_scheme).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    Some((
        Link {
            url: raw.to_owned(),
            host,
        },
        url,
    ))
}

pub fn plain_text(markdown_text: &str) -> String {
    markdown::plain_text(markdown_text)
}

pub fn words(text: &str) -> Vec<String> {
    text.unicode_words().map(str::to_owned).collect()
}

pub fn sentences(text: &str) -> Vec<String> {
    text.unicode_sentences()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Words that mix writing systems, like Latin with one Cyrillic letter.
pub fn mixed_script_words(text: &str) -> Vec<String> {
    text.unicode_words()
        .filter(|w| !w.is_single_script())
        .map(str::to_owned)
        .collect()
}

/// The writing systems used, by name ("Latin", "Cyrillic"), most used
/// first. Digits, punctuation and emoji belong to none.
pub fn script_mix(text: &str) -> Vec<String> {
    let mut counts: HashMap<Script, usize> = HashMap::new();
    for c in text.chars() {
        let script = c.script();
        if !matches!(script, Script::Common | Script::Inherited | Script::Unknown) {
            *counts.entry(script).or_default() += 1;
        }
    }
    let mut scripts: Vec<(Script, usize)> = counts.into_iter().collect();
    scripts.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.full_name().cmp(b.0.full_name()))
    });
    scripts
        .into_iter()
        .map(|(s, _)| s.full_name().to_owned())
        .collect()
}

/// 0.0 (nothing alike) to 1.0 (the same), after normalizing both.
pub fn similarity(a: &str, b: &str) -> f64 {
    strsim::normalized_levenshtein(&normalize(a), &normalize(b))
}

/// Share of letters that are capitals; 0.0 with no letters.
pub fn caps_ratio(text: &str) -> f64 {
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    if letters == 0 {
        return 0.0;
    }
    text.chars().filter(|c| c.is_uppercase()).count() as f64 / letters as f64
}

/// Share of non-space characters that repeat the one before at least twice
/// ("!!!", "sooo"); 0.0 for empty text.
pub fn repeat_ratio(text: &str) -> f64 {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return 0.0;
    }
    let repeats = chars
        .windows(3)
        .filter(|w| w[0] == w[1] && w[1] == w[2])
        .count();
    repeats as f64 / chars.len() as f64
}

/// Emoji, counted as what a reader sees: a family or a flag is one.
pub fn emoji_count(text: &str) -> usize {
    text.graphemes(true)
        .filter(|g| g.chars().any(is_emoji))
        .count()
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        // Pictographs, emoticons, transport, symbols and the regional
        // indicators that make flags.
        0x1F000..=0x1FAFF
        // Miscellaneous symbols, dingbats, and arrows and stars used as emoji.
        | 0x2600..=0x27BF
        | 0x2B00..=0x2BFF
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(words: &[&str]) -> WordList {
        WordList::new(words).unwrap()
    }

    #[test]
    fn regexes_are_case_insensitive_and_cached() {
        let cache = TextCache::default();
        let re = cache.regex("spam").unwrap();
        assert!(is_match(&re, "SPAM here"));
        assert!(Arc::ptr_eq(&re, &cache.regex("spam").unwrap()));
        assert!(!is_match(&cache.regex("(?-i)spam").unwrap(), "SPAM"));
    }

    #[test]
    fn bad_patterns_are_errors() {
        assert!(
            compile("(unclosed")
                .unwrap_err()
                .to_string()
                .contains("invalid pattern")
        );
    }

    #[test]
    fn matches_report_character_positions() {
        let re = compile(r"\d+").unwrap();
        let found = find_all(&re, "né 12 et 345");
        assert_eq!(
            found[0],
            Found {
                text: "12".into(),
                start: 3,
                end: 5
            }
        );
        assert_eq!(
            found[1],
            Found {
                text: "345".into(),
                start: 9,
                end: 12
            }
        );
        assert_eq!(find(&re, "none"), None);
    }

    #[test]
    fn captures_by_name_and_number() {
        let re = compile(r"(?<user>\w+)@(\w+)").unwrap();
        let caps = captures(&re, "mail me: pat@example").unwrap();
        assert_eq!(caps["user"], "pat");
        assert_eq!(caps["1"], "pat");
        assert_eq!(caps["2"], "example");
        assert_eq!(caps["0"], "pat@example");
    }

    #[test]
    fn replace_and_split() {
        let re = compile(r"\s*,\s*").unwrap();
        assert_eq!(split(&re, "a , b,c"), ["a", "b", "c"]);
        assert_eq!(
            replace(&compile(r"(\w+)@").unwrap(), "pat@x", "[$1]"),
            "[pat]x"
        );
    }

    #[test]
    fn backtracking_patterns_stay_fast() {
        let re = compile("^(a+)+$").unwrap();
        let text = "a".repeat(50_000) + "!";
        let started = std::time::Instant::now();
        assert!(!is_match(&re, &text));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn word_lists_see_through_evasions() {
        let banned = list(&["spam", "free crypto", "paypal"]);
        assert_eq!(banned.find_in("Buy S P A M now"), ["spam"]);
        assert_eq!(banned.find_in("spaaaam!"), ["spam"]);
        assert_eq!(banned.find_in("s.p.a.m"), ["spam"]);
        assert_eq!(banned.find_in("FREE crypto here"), ["free crypto"]);
        assert_eq!(banned.find_in("F R E E crypto"), ["free crypto"]);
        assert_eq!(banned.find_in("s p a a m"), ["spam"]);
        assert_eq!(banned.find_in("log in to p\u{0430}yp\u{0430}l"), ["paypal"]);
        assert_eq!(banned.find_in("sp\u{200B}am"), ["spam"]);
        assert!(banned.find_in("a spammer's lament").is_empty());
    }

    #[test]
    fn word_lists_report_each_entry_once_in_list_order() {
        let banned = list(&["bee", "ant"]);
        assert_eq!(banned.find_in("ant bee ant bee"), ["bee", "ant"]);
    }

    #[test]
    fn a_failed_long_entry_does_not_hide_a_short_one() {
        // "free crypto" starts at the same place as "free" but is followed by
        // a letter, so only "free" is a whole-word match.
        let banned = list(&["free cryptos", "free crypto", "free"]);
        assert_eq!(banned.find_in("free cryptosx"), ["free"]);
    }

    #[test]
    fn word_lists_ignore_blank_entries() {
        let banned = list(&["", "  ", "spam"]);
        assert_eq!(banned.len(), 1);
        assert!(list(&[] as &[&str]).find_in("anything").is_empty());
    }

    #[test]
    fn links_from_markdown_and_bare_text() {
        let found = links(
            "see [rules](https://www.Example.com/x), evil.biz/win and https://www.example.com/x",
        );
        let hosts: Vec<_> = found.iter().map(|l| l.host.as_str()).collect();
        assert_eq!(hosts, ["example.com", "evil.biz"]);
        assert!(links("write to pat@example.com").is_empty());
        assert!(links("[x](javascript:alert(1))").is_empty());
    }

    #[test]
    fn structure() {
        assert_eq!(words("Two words."), ["Two", "words"]);
        assert_eq!(sentences("One. Two? Three!"), ["One.", "Two?", "Three!"]);
        assert_eq!(plain_text("**bold** text"), "bold text");
    }

    #[test]
    fn scripts() {
        assert_eq!(mixed_script_words("hello p\u{0430}ypal"), ["p\u{0430}ypal"]);
        assert!(mixed_script_words("Привет hello").is_empty());
        assert_eq!(script_mix("hello there Привет 123"), ["Latin", "Cyrillic"]);
    }

    #[test]
    fn measures() {
        assert!(similarity("A lighthouse", "a lighthouse!") > 0.9);
        assert_eq!(caps_ratio("ABC def"), 0.5);
        assert_eq!(caps_ratio("123"), 0.0);
        assert!(repeat_ratio("noooooo!!!!") > 0.5);
        assert_eq!(repeat_ratio("plain words"), 0.0);
        assert_eq!(emoji_count("hi 👋🏽 🇬🇧 👨‍👩‍👧"), 3);
    }

    #[test]
    fn normalizing() {
        assert_eq!(normalize("Café\u{200B}"), "cafe");
        assert_eq!(normalize("ﬁne"), "fine");
        assert_eq!(skeleton("p\u{0430}ypal"), skeleton("paypal"));
        assert_eq!(squash("s p a m and ok"), "spam and ok");
    }
}
