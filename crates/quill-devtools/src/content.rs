//! Built-in word lists and templates for demo prompts. Nothing is downloaded.

use quill_proto::board::{Maturity, PromptState};

use crate::rng::Rng;

pub const LOGIN_NAMES: &[&str] = &[
    "ash", "briar", "corin", "dell", "ember", "fen", "gale", "hollis", "iris", "juno", "kestrel",
    "linden", "marlow", "nell", "oren", "pell", "quinn", "rowan", "sable", "tamsin",
];

const HANDLE_ADJECTIVES: &[&str] = &[
    "ink", "quiet", "salt", "amber", "moss", "cinder", "paper", "lantern", "river", "hollow",
    "brass", "velvet", "north", "rook", "thistle", "juniper", "copper", "fog",
];

const HANDLE_NOUNS: &[&str] = &[
    "fox", "wren", "moth", "heron", "otter", "crane", "hare", "finch", "lark", "pike", "vole",
    "badger", "owl", "newt",
];

const SUMMARIES: &[&str] = &[
    "Slow burns, unreliable narrators, too much tea.",
    "Mostly horror. Occasionally kind.",
    "Writing between night shifts.",
    "Looking for co-writers who like maps and footnotes.",
    "Poetry when it rains, prose when it doesn't.",
];

const ADJECTIVES: &[&str] = &[
    "drowned",
    "borrowed",
    "silver",
    "quiet",
    "burning",
    "paper",
    "forgotten",
    "crooked",
    "last",
    "hollow",
    "midnight",
    "glass",
    "winter",
    "stolen",
    "patient",
];

const NOUNS: &[&str] = &[
    "ledger",
    "lighthouse",
    "orchard",
    "clockmaker",
    "map",
    "choir",
    "ferry",
    "archive",
    "garden",
    "signal",
    "tower",
    "key",
    "comet",
    "inn",
    "violin",
];

const PLACES: &[&str] = &[
    "Harrow Point",
    "the salt flats",
    "Low Market",
    "the night train",
    "Station Nine",
    "the river delta",
    "Kettle Street",
    "the old observatory",
    "Port Ansel",
    "the border town",
];

const PEOPLE: &[&str] = &[
    "the cartographer",
    "a retired detective",
    "the second daughter",
    "an apprentice witch",
    "the night nurse",
    "a lighthouse keeper",
    "the understudy",
    "an estranged brother",
    "the last ferryman",
    "a ghost in the choir",
];

const EVENTS: &[&str] = &[
    "flood",
    "coronation",
    "last broadcast",
    "harvest",
    "eclipse",
    "election",
    "wedding",
];

const THINGS: &[&str] = &[
    "a letter in their own handwriting",
    "a key with no lock",
    "a photograph of a stranger's wedding",
    "a map with one street missing",
    "a recording of tomorrow's news",
    "a seed that hums",
];

const PERIODS: &[&str] = &["winter", "year", "full moon", "Tuesday"];

const CONSTRAINTS: &[&str] = &[
    "No dialogue until the halfway point.",
    "Every scene happens at night.",
    "One character only ever speaks in questions.",
    "The ending has to be hopeful, even if nothing else is.",
    "Alternate narrators each turn.",
    "Somebody is lying, and the reader should guess who.",
];

/// User-created theme tags, used when the server allows them.
pub const THEMES: &[&str] = &[
    "Lighthouses",
    "Found family",
    "Time loops",
    "Small towns",
    "Revenge",
    "First contact",
    "Heists",
    "Grief",
    "Dragons",
    "Letters",
    "Secret societies",
    "Road trips",
];

const GENRES: &[&str] = &[
    "fantasy",
    "science-fiction",
    "horror",
    "mystery",
    "romance",
    "thriller",
    "historical",
    "literary",
    "comedy",
    "slice-of-life",
];
const FORMATS: &[&str] = &["prose", "poetry", "script", "epistolary", "role-play"];
const LENGTHS: &[&str] = &["flash", "short", "long", "open-ended"];
const MILD_WARNINGS: &[&str] = &["violence", "death"];
const WARNINGS: &[&str] = &[
    "violence",
    "gore",
    "death",
    "abuse",
    "self-harm",
    "substance-use",
];

fn cap(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// Distinct persona handles and display names, as many as asked for. None
/// is reserved by default (see the test below), so seeding never fails on one.
pub fn handles(rng: &mut Rng, n: usize) -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = HANDLE_ADJECTIVES
        .iter()
        .flat_map(|a| {
            HANDLE_NOUNS
                .iter()
                .map(move |n| (format!("{a}_{n}"), format!("{} {}", cap(a), cap(n))))
        })
        .collect();
    let mut out = Vec::with_capacity(n);
    while out.len() < n && !all.is_empty() {
        out.push(all.swap_remove(rng.below(all.len())));
    }
    out
}

pub fn summary(rng: &mut Rng) -> Option<String> {
    rng.chance(50).then(|| (*rng.pick(SUMMARIES)).to_owned())
}

pub fn title(rng: &mut Rng) -> String {
    let (adj, noun, place, person, event) = (
        *rng.pick(ADJECTIVES),
        *rng.pick(NOUNS),
        *rng.pick(PLACES),
        *rng.pick(PEOPLE),
        *rng.pick(EVENTS),
    );
    match rng.below(10) {
        0 => format!("The {adj} {noun}"),
        1 => format!("{} at {place}", cap(noun)),
        2 => format!("What the {noun} remembered"),
        3 => format!("Letters from {place}"),
        4 => format!("The last {noun} in {place}"),
        5 => format!("A {noun} for {person}"),
        6 => format!("Before the {event}"),
        7 => format!("{} and the {adj} {noun}", cap(person)),
        8 => format!("Seven days in {place}"),
        _ => format!("Nobody asked the {noun}"),
    }
}

const SENTENCE_TEMPLATES: usize = 10;

fn sentence(rng: &mut Rng, template: usize) -> String {
    let (place, person, event, thing, period) = (
        *rng.pick(PLACES),
        *rng.pick(PEOPLE),
        *rng.pick(EVENTS),
        *rng.pick(THINGS),
        *rng.pick(PERIODS),
    );
    match template {
        0 => format!("{} finds {thing} that shouldn't exist.", cap(person)),
        1 => format!(
            "Every {period}, {person} returns to {place}, and every {period} something is different."
        ),
        2 => format!("Nobody in {place} talks about the {event}."),
        3 => format!("Write it from the point of view of {person}."),
        4 => "The story should end before anyone learns the whole truth.".to_owned(),
        5 => "Start with a door that won't stay shut.".to_owned(),
        6 => format!("*{}* is the only clue.", cap(thing)),
        7 => "I'm looking for a co-writer who likes slow builds.".to_owned(),
        8 => "Bonus points for **weather** that behaves like a character.".to_owned(),
        _ => format!("It begins the morning after the {event}."),
    }
}

/// A Markdown body of one to three paragraphs, sometimes with a list.
pub fn body(rng: &mut Rng) -> String {
    // Each sentence template is used at most once per prompt.
    let mut templates = rng.pick_distinct(&(0..SENTENCE_TEMPLATES).collect::<Vec<_>>(), 9);
    let mut paragraphs = Vec::new();
    for _ in 0..rng.between(1, 3) {
        let n = rng.between(2, 4).min(templates.len());
        if n == 0 {
            break;
        }
        let sentences: Vec<String> = templates
            .drain(..n)
            .map(|template| sentence(rng, template))
            .collect();
        paragraphs.push(sentences.join(" "));
    }
    if rng.chance(25) {
        let items: Vec<String> = {
            let n = rng.between(2, 3);
            rng.pick_distinct(CONSTRAINTS, n)
        }
        .into_iter()
        .map(|c| format!("- {c}"))
        .collect();
        paragraphs.push(format!("Some constraints:\n\n{}", items.join("\n")));
    }
    paragraphs.join("\n\n")
}

pub fn maturity(rng: &mut Rng) -> Maturity {
    match rng.below(100) {
        0..35 => Maturity::General,
        35..70 => Maturity::Teen,
        70..90 => Maturity::Mature,
        _ => Maturity::Explicit,
    }
}

pub fn state(rng: &mut Rng) -> PromptState {
    match rng.below(100) {
        0..70 => PromptState::Open,
        70..85 => PromptState::Busy,
        _ => PromptState::Closed,
    }
}

/// Tags as `namespace:name`, consistent with the maturity rating.
pub fn tags(rng: &mut Rng, maturity: Maturity, user_tags: bool) -> Vec<String> {
    let mut tags: Vec<String> = {
        let n = if rng.chance(30) { 2 } else { 1 };
        rng.pick_distinct(GENRES, n)
    }
    .into_iter()
    .map(|g| format!("genre:{g}"))
    .collect();
    tags.push(format!("format:{}", rng.pick(FORMATS)));
    tags.push(format!("length:{}", rng.pick(LENGTHS)));
    let warnings = match maturity {
        Maturity::General => Vec::new(),
        Maturity::Teen => {
            let n = rng.below(2);
            rng.pick_distinct(MILD_WARNINGS, n)
        }
        Maturity::Mature => {
            let n = rng.below(3);
            rng.pick_distinct(WARNINGS, n)
        }
        Maturity::Explicit => {
            let mut w = {
                let n = rng.below(2);
                rng.pick_distinct(WARNINGS, n)
            };
            if rng.chance(70) {
                w.push("sexual-content");
            }
            w
        }
    };
    tags.extend(warnings.into_iter().map(|w| format!("content-warning:{w}")));
    if user_tags {
        tags.extend(
            {
                let n = rng.below(3);
                rng.pick_distinct(THEMES, n)
            }
            .into_iter()
            .map(|t| format!("theme:{t}")),
        );
    }
    tags
}

#[cfg(test)]
mod tests {
    use quill_proto::handles::{DEFAULT_EXACT, DEFAULT_WORDS, ReservedKind, is_reserved};

    use super::*;

    #[test]
    fn no_generated_handle_is_reserved_by_default() {
        let entries: Vec<(ReservedKind, &str)> = DEFAULT_WORDS
            .iter()
            .map(|w| (ReservedKind::Word, *w))
            .chain(DEFAULT_EXACT.iter().map(|e| (ReservedKind::Exact, *e)))
            .collect();
        let every = HANDLE_ADJECTIVES.len() * HANDLE_NOUNS.len();
        let all = handles(&mut Rng::new(1), every);
        assert_eq!(all.len(), every);
        for (handle, _) in all {
            assert!(!is_reserved(&handle, &entries), "{handle} is reserved");
        }
    }
}
