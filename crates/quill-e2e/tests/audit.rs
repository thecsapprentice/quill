//! The database audit (2l) covers the whole schema: every column of every
//! table is listed in `docs/server-data.md` with a class, and nothing listed
//! there is gone. A migration that adds a column must say what it holds.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use quill_e2e::TestServer;

const CLASSES: [&str; 7] = [
    "public",
    "login",
    "moderation",
    "metadata",
    "ciphertext",
    "digest",
    "settings",
];

/// sqlx's own bookkeeping, described as a whole in the audit.
const WHOLE_TABLES: [&str; 1] = ["_sqlx_migrations"];

fn strip_ticks(cell: &str) -> Option<&str> {
    cell.trim().strip_prefix('`')?.strip_suffix('`')
}

/// `(table, column) → class` from the audit's "Every column" section.
fn audited() -> BTreeMap<(String, String), String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/server-data.md");
    let text = std::fs::read_to_string(path).unwrap();
    let start = text
        .find("## Every column")
        .expect("the audit lists every column");
    let end = text[start..]
        .find("\n## ")
        .map_or(text.len(), |i| start + i);
    let mut out = BTreeMap::new();
    let mut table: Option<String> = None;
    for line in text[start..end].lines() {
        if let Some(h) = line.strip_prefix("### ") {
            table = strip_ticks(h).map(str::to_owned);
            continue;
        }
        let cells: Vec<&str> = line.split('|').collect();
        if cells.len() < 4 {
            continue;
        }
        let (Some(column), Some(class)) = (strip_ticks(cells[1]), strip_ticks(cells[2])) else {
            continue;
        };
        let table = table.clone().expect("a column under a table heading");
        assert!(
            CLASSES.contains(&class),
            "{table}.{column}: unknown class `{class}`"
        );
        let what = cells[3].trim();
        assert!(!what.is_empty(), "{table}.{column}: say what it holds");
        let previous = out.insert((table.clone(), column.to_owned()), class.to_owned());
        assert!(previous.is_none(), "{table}.{column} is listed twice");
    }
    out
}

#[tokio::test]
async fn the_audit_covers_every_column() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let db = server.db().await.unwrap();
    let columns: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.table_schema::text, c.table_name::text, c.column_name::text
         FROM information_schema.columns c
         JOIN information_schema.tables t
           ON t.table_schema = c.table_schema AND t.table_name = c.table_name
         WHERE t.table_type = 'BASE TABLE'
           AND c.table_schema NOT IN ('pg_catalog', 'information_schema')
         ORDER BY 1, 2, c.ordinal_position",
    )
    .fetch_all(&db)
    .await
    .unwrap();
    assert!(
        columns.iter().all(|(schema, _, _)| schema == "public"),
        "every table is in the public schema"
    );
    let actual: BTreeSet<(String, String)> = columns
        .into_iter()
        .filter(|(_, t, _)| !WHOLE_TABLES.contains(&t.as_str()))
        .map(|(_, t, c)| (t, c))
        .collect();
    let listed = audited();
    let listed_keys: BTreeSet<(String, String)> = listed.keys().cloned().collect();

    let missing: Vec<String> = actual
        .difference(&listed_keys)
        .map(|(t, c)| format!("{t}.{c}"))
        .collect();
    let stale: Vec<String> = listed_keys
        .difference(&actual)
        .map(|(t, c)| format!("{t}.{c}"))
        .collect();
    assert!(
        missing.is_empty(),
        "columns not in docs/server-data.md (add each with its class and what it holds): {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "docs/server-data.md lists columns that are gone: {stale:?}"
    );
    // Every table holding writers' content keeps it as ciphertext.
    for (table, column) in [
        ("group_messages", "data"),
        ("blobs", "data"),
        ("backup_parts", "data"),
        ("session_requests", "pitch"),
    ] {
        assert_eq!(
            listed[&(table.to_owned(), column.to_owned())],
            "ciphertext",
            "{table}.{column}"
        );
    }
    server.stop().await.unwrap();
}
