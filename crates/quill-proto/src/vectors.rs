//! Test vectors (protocol draft, section 11) for the formats this crate
//! defines: tag and channel names, and padding and size limits. They are
//! generated here, committed under `test-vectors/`, and checked two ways:
//! regenerating them must reproduce the committed files byte for byte, and
//! every committed case must give its recorded result through the public
//! functions.
//!
//! To rewrite the files after a deliberate change:
//! `QUILL_WRITE_VECTORS=1 cargo test -p quill-proto vectors`.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};

use crate::{limits::*, validate};

fn dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors")
}

/// Regenerates a file's contents and compares (or writes) them.
fn check_file(name: &str, value: &Value) -> Value {
    let path = dir().join(name);
    let text = serde_json::to_string_pretty(value).unwrap() + "\n";
    if std::env::var_os("QUILL_WRITE_VECTORS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} missing; set QUILL_WRITE_VECTORS=1", path.display()));
    assert_eq!(
        committed, text,
        "{name} differs from what the code generates"
    );
    serde_json::from_str(&committed).unwrap()
}

/// `U+0063 U+0301 …`, so invisible and lookalike characters can be read.
fn codepoints(s: &str) -> String {
    s.chars()
        .map(|c| format!("U+{:04X}", u32::from(c)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn named(input: &str) -> Value {
    json!({ "input": input, "codepoints": codepoints(input) })
}

// ---- Tag and channel names -------------------------------------------------------------------

const SLUGS_OK: [(&str, &str); 14] = [
    ("Slow burn", "folding case, a space becomes a hyphen"),
    (
        "  Café  Noir!! ",
        "an accent folds; runs of spaces and punctuation are one hyphen; ends are trimmed",
    ),
    (
        "Cafe\u{301} Noir",
        "a combining accent (decomposed form) folds the same way",
    ),
    (
        "Sci_Fi / Space-Opera",
        "underscores, slashes and hyphens all separate words",
    ),
    ("Writer's block", "an ASCII apostrophe joins words"),
    (
        "Writer\u{2019}s block",
        "a typographic apostrophe joins words too",
    ),
    ("Straße", "ß becomes ss"),
    (
        "Łódź",
        "letters without a decomposition are folded by table",
    ),
    ("Ærøskøbing", "æ and ø"),
    ("Þingvellir", "thorn becomes th"),
    ("\u{FB01}re", "a ligature decomposes (NFKD)"),
    (
        "ＦＵＬＬ\u{3000}ｗｉｄｔｈ",
        "full-width letters and an ideographic space",
    ),
    ("1920s", "digits stay"),
    ("a\u{2003}b", "any whitespace separates words"),
];

const SLUGS_BAD: [(&str, &str); 9] = [
    ("怪談", "another script is rejected, not dropped"),
    ("\u{0430}pple", "a Cyrillic а that looks Latin"),
    ("horror 🎃", "an emoji"),
    (
        "x\u{200B}y",
        "a zero-width space is not whitespace or punctuation",
    ),
    ("!!!", "nothing left"),
    ("", "empty"),
    ("   ", "only spaces"),
    ("ab\u{0}c", "a control character"),
    ("ω", "Greek"),
];

fn names() -> Value {
    let slug_ok: Vec<Value> = SLUGS_OK
        .iter()
        .map(|(input, why)| {
            let mut v = named(input);
            v["slug"] = json!(validate::tag_slug(input).unwrap());
            v["why"] = json!(why);
            v
        })
        .collect();
    let mut slug_bad: Vec<Value> = SLUGS_BAD
        .iter()
        .map(|(input, why)| {
            let mut v = named(input);
            v["why"] = json!(why);
            v
        })
        .collect();
    let at = "a".repeat(TAG_SLUG_MAX_CHARS);
    let over = "a".repeat(TAG_SLUG_MAX_CHARS + 1);
    slug_bad.push(json!({ "input": over, "why": "one character over the limit" }));
    let mut long_ok = named(&at);
    long_ok["slug"] = json!(validate::tag_slug(&at).unwrap());
    long_ok["why"] = json!("exactly at the limit");
    // Folding can lengthen a name: 20 × ß is 40 letters once folded.
    let folded = "ß".repeat(TAG_SLUG_MAX_CHARS / 2);
    let mut folded_ok = named(&folded);
    folded_ok["slug"] = json!(validate::tag_slug(&folded).unwrap());
    folded_ok["why"] = json!("the limit counts the slug, after folding");
    let mut slug_ok = slug_ok;
    slug_ok.push(long_ok);
    slug_ok.push(folded_ok);
    slug_bad.push(json!({ "input": "ß".repeat(TAG_SLUG_MAX_CHARS / 2 + 1), "why": "over the limit once folded" }));

    let channel_ok = ["story", "planning", "world-notes", "act-2", "a"]
        .iter()
        .map(|s| json!(s))
        .chain(std::iter::once(json!("x".repeat(CHANNEL))))
        .collect::<Vec<_>>();
    let channel_bad: Vec<Value> = [
        ("", "empty"),
        (
            "Story",
            "uppercase is not folded: a name means the same to every client",
        ),
        ("two words", "a space"),
        ("-x", "starts with a hyphen"),
        ("x-", "ends with a hyphen"),
        ("a--b", "two hyphens together"),
        ("café", "not ASCII"),
        ("a_b", "an underscore"),
    ]
    .iter()
    .map(|(input, why)| {
        let mut v = named(input);
        v["why"] = json!(why);
        v
    })
    .chain(std::iter::once(
        json!({ "input": "x".repeat(CHANNEL + 1), "why": "one character over the limit" }),
    ))
    .collect();

    let namespace_ok: Vec<Value> = [("Content-Warning", "content-warning"), ("genre", "genre")]
        .iter()
        .map(|(i, o)| json!({ "input": i, "namespace": o }))
        .collect();
    json!({
        "description": "Tag slugs (validate::tag_slug): NFKD, combining marks dropped, a few letters folded by table (ß ss, æ ae, œ oe, ø o, đ ð d, ł l, þ th, ı i), ASCII letters lowercased, whitespace and ASCII punctuation separate words with one hyphen, apostrophes (U+0027, U+2019) join words, and anything else rejects the whole name. At most 40 characters after folding. Channel names (validate::channel) are checked as given, never folded: 1-32 of a-z, 0-9 and single inner hyphens; a note whose channel is malformed is filed under story. Tag namespaces are lowercased, then 1-32 of a-z, 0-9 and single inner hyphens.",
        "tagSlug": { "valid": slug_ok, "invalid": slug_bad },
        "channel": { "valid": channel_ok, "invalid": channel_bad },
        "namespace": { "valid": namespace_ok, "invalid": ["-cw", "c w", "cw--x", "", "ns:x"] },
    })
}

const CHANNEL: usize = validate::CHANNEL_MAX_CHARS;

#[test]
fn name_vectors() {
    let v = check_file("names.json", &names());
    for case in v["tagSlug"]["valid"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            validate::tag_slug(input).unwrap(),
            case["slug"],
            "{input:?}"
        );
    }
    for case in v["tagSlug"]["invalid"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        assert!(validate::tag_slug(input).is_err(), "{input:?}");
    }
    for case in v["channel"]["valid"].as_array().unwrap() {
        assert!(validate::channel(case.as_str().unwrap()).is_ok(), "{case}");
    }
    for case in v["channel"]["invalid"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        assert!(validate::channel(input).is_err(), "{input:?}");
    }
    for case in v["namespace"]["valid"].as_array().unwrap() {
        assert_eq!(
            validate::namespace(case["input"].as_str().unwrap()).unwrap(),
            case["namespace"]
        );
    }
    for case in v["namespace"]["invalid"].as_array().unwrap() {
        assert!(
            validate::namespace(case.as_str().unwrap()).is_err(),
            "{case}"
        );
    }
}

// ---- Padding and limits ----------------------------------------------------------------------

/// A text field's rule, applied to `input`.
fn field_ok(field: &str, input: &str) -> bool {
    match field {
        "username" => validate::username(input).is_ok(),
        "password" => validate::password(input).is_ok(),
        "displayName" => validate::display_name(Some(input)).is_ok(),
        "summary" => validate::summary(Some(input)).is_ok(),
        "promptTitle" => validate::prompt_title(input).is_ok(),
        "promptBody" => validate::prompt_body(input).is_ok(),
        "reportDetails" => validate::report_details(Some(input)).is_ok(),
        "tagDisplay" => validate::tag_display(input).is_ok(),
        "namespace" => validate::namespace(input).is_ok(),
        "tagSlug" => validate::tag_slug(input).is_ok(),
        "channel" => validate::channel(input).is_ok(),
        other => panic!("no rule for {other}"),
    }
}

/// `(field, repeated text, limit, counted in)`. Multi-byte text shows
/// which limits count characters and which count bytes.
const TEXT_LIMITS: [(&str, &str, usize, &str); 12] = [
    ("username", "a", USERNAME_MAX_CHARS, "characters"),
    ("password", "é", PASSWORD_MAX_BYTES, "bytes"),
    ("password", "a", PASSWORD_MAX_BYTES, "bytes"),
    ("displayName", "é", DISPLAY_NAME_MAX_CHARS, "characters"),
    ("summary", "é", SUMMARY_MAX_CHARS, "characters"),
    ("promptTitle", "é", PROMPT_TITLE_MAX_CHARS, "characters"),
    ("promptBody", "é", PROMPT_BODY_MAX_CHARS, "characters"),
    ("reportDetails", "é", REPORT_DETAILS_MAX_CHARS, "characters"),
    ("tagDisplay", "é", TAG_DISPLAY_MAX_CHARS, "characters"),
    ("namespace", "a", NAMESPACE_MAX_CHARS, "characters"),
    ("tagSlug", "a", TAG_SLUG_MAX_CHARS, "characters"),
    ("channel", "a", CHANNEL, "characters"),
];

fn limits() -> Value {
    let text: Vec<Value> = TEXT_LIMITS
        .iter()
        .map(|(field, unit, limit, counted)| {
            // How many times `unit` fits exactly at the limit.
            let times = if *counted == "bytes" {
                limit / unit.len()
            } else {
                *limit
            };
            json!({
                "field": field, "repeat": unit, "times": times,
                "limit": limit, "countedIn": counted,
                "atLimit": field_ok(field, &unit.repeat(times)),
                "overLimit": field_ok(field, &unit.repeat(times + 1)),
            })
        })
        .collect();
    let max = DEFAULT_MAX_ATTACHMENT_BYTES;
    let buckets: Vec<Value> = [
        (0, max),
        (1, max),
        (4096, max),
        (4097, max),
        (65_536, max),
        (65_537, max),
        (max - 1, max),
        (max, max),
        (max + 1, max),
        (5000, 6000),
        (6000, 6000),
        (6001, 6000),
    ]
    .iter()
    .map(|(len, max)| json!({ "length": len, "max": max, "bucket": blob_bucket(*len, *max) }))
    .collect();
    let not_buckets: Vec<Value> = [
        (0, max),
        (1000, max),
        (4095, max),
        (5000, max),
        (max * 2, max),
    ]
    .iter()
    .map(|(len, max)| json!({ "length": len, "max": max }))
    .collect();
    let tags_at: Vec<String> = (0..PROMPT_MAX_TAGS)
        .map(|i| format!("theme:t{i}"))
        .collect();
    json!({
        "description": "Default limits (an instance may change them and publishes its own in GET /api/v1/instance). Text limits: each field's rule applied to `repeat` written `times` times, exactly at the limit (atLimit), and once more (overLimit); limits count Unicode scalar values unless countedIn says bytes. Blob buckets (limits::blob_bucket): the length of a sealed attachment, AES-GCM tag included, is padded with zeros to the smallest power of two from 4096 that holds it, or to the maximum when that is smaller; a length over the maximum has no bucket (null). Servers refuse uploads whose length is not a bucket (notBuckets). MLS application messages are padded to a multiple of mlsPaddingBytes (OpenMLS padding_size).",
        "defaults": {
            "maxMessageBytes": DEFAULT_MAX_MESSAGE_BYTES,
            "maxEnvelopeBytes": DEFAULT_MAX_ENVELOPE_BYTES,
            "maxAttachmentBytes": DEFAULT_MAX_ATTACHMENT_BYTES,
            "maxBackupBytes": DEFAULT_MAX_BACKUP_BYTES,
            "backupPartBytes": BACKUP_PART_BYTES,
            "accountQuotaBytes": DEFAULT_ACCOUNT_QUOTA_BYTES,
            "retentionDays": DEFAULT_RETENTION_DAYS,
            "requestExpiryDays": DEFAULT_REQUEST_EXPIRY_DAYS,
            "messagesPerMinute": DEFAULT_MESSAGES_PER_MINUTE,
            "promptMaxTags": PROMPT_MAX_TAGS,
            "searchQueryMaxChars": SEARCH_QUERY_MAX_CHARS,
            "pageSize": DEFAULT_PAGE_SIZE,
            "maxPageSize": MAX_PAGE_SIZE,
            "mlsPaddingBytes": 256,
            "blobMinBucket": BLOB_MIN_BUCKET,
        },
        "text": text,
        "passwordMinimum": {
            "minChars": PASSWORD_MIN_CHARS,
            "repeat": "é",
            "atMinimum": validate::password(&"é".repeat(PASSWORD_MIN_CHARS)).is_ok(),
            "underMinimum": validate::password(&"é".repeat(PASSWORD_MIN_CHARS - 1)).is_ok(),
        },
        "promptTags": {
            "atLimit": validate::tag_specs(&tags_at).is_ok(),
            "overLimit": validate::tag_specs(&[tags_at.clone(), vec!["theme:extra".into()]].concat()).is_ok(),
            "duplicatesCountOnce": validate::tag_specs(&[tags_at, vec!["theme:T0".into()]].concat()).is_ok(),
        },
        "blobBuckets": buckets,
        "notBuckets": not_buckets,
    })
}

#[test]
fn limit_vectors() {
    let v = check_file("limits.json", &limits());
    for case in v["text"].as_array().unwrap() {
        let field = case["field"].as_str().unwrap();
        let unit = case["repeat"].as_str().unwrap();
        let times = usize::try_from(case["times"].as_u64().unwrap()).unwrap();
        assert!(case["atLimit"].as_bool().unwrap(), "{field} at its limit");
        assert!(!case["overLimit"].as_bool().unwrap(), "{field} over");
        assert!(field_ok(field, &unit.repeat(times)), "{field}");
        assert!(!field_ok(field, &unit.repeat(times + 1)), "{field}");
    }
    let min = &v["passwordMinimum"];
    let n = usize::try_from(min["minChars"].as_u64().unwrap()).unwrap();
    assert_eq!(min["atMinimum"], true);
    assert_eq!(min["underMinimum"], false);
    assert!(validate::password(&"é".repeat(n)).is_ok());
    assert!(validate::password(&"é".repeat(n - 1)).is_err());
    assert_eq!(v["promptTags"]["atLimit"], true);
    assert_eq!(v["promptTags"]["overLimit"], false);
    assert_eq!(v["promptTags"]["duplicatesCountOnce"], true);
    for case in v["blobBuckets"].as_array().unwrap() {
        let (len, max) = (
            case["length"].as_u64().unwrap(),
            case["max"].as_u64().unwrap(),
        );
        assert_eq!(blob_bucket(len, max), case["bucket"].as_u64(), "{len}");
        if let Some(b) = case["bucket"].as_u64() {
            assert!(is_blob_bucket(b, max), "{b}");
        }
    }
    for case in v["notBuckets"].as_array().unwrap() {
        let (len, max) = (
            case["length"].as_u64().unwrap(),
            case["max"].as_u64().unwrap(),
        );
        assert!(!is_blob_bucket(len, max), "{len}");
    }
}
