//! The Phase 2 gate (2l): the server never holds plaintext or private keys.
//!
//! One test drives everything Phase 2 built with a random canary in all a
//! writer writes, then searches a dump of every table in the database, and
//! the server's logs, for the canary and for every secret the clients hold,
//! in each encoding they could be stored in. Positive controls show the
//! search finds what is meant to be there.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    io::Write,
    sync::{Arc, Mutex},
    time::Duration,
};

use quill_core::{
    Client, Store, backup, devices, keys, requests,
    sessions::{self, Draft},
    testing::{self, Secret},
};
use quill_e2e::{TEST_PASSWORD, TestServer};
use quill_proto::{
    api::{CreatePersona, Credentials, Persona},
    board::{CreatePrompt, Maturity, PromptState},
};
use sqlx::PgPool;
use uuid::Uuid;

// ---- Capturing the server's logs ----------------------------------------------------

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Logs everything the server and the libraries it serves with emit, at the
/// most detailed level. (Client libraries are left out: a client's own logs
/// stay on its device.)
fn capture_logs() -> Logs {
    let logs = Logs::default();
    let writer = logs.clone();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "quill_server=trace,sqlx=trace,axum=trace,tower=trace,tower_http=trace,hyper=trace,h2=trace",
        ))
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .init();
    logs
}

// ---- Dumping the database -------------------------------------------------------

/// Every row of every table, as JSON text (bytea as `\x` hex), table by
/// table. Tables are found from the schema, so new ones are covered.
async fn dump(db: &PgPool) -> String {
    let tables: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_schema::text, table_name::text FROM information_schema.tables
         WHERE table_type = 'BASE TABLE'
           AND table_schema NOT IN ('pg_catalog', 'information_schema')
         ORDER BY 1, 2",
    )
    .fetch_all(db)
    .await
    .unwrap();
    assert!(tables.len() > 20, "the schema has its tables: {tables:?}");
    let mut out = String::new();
    for (schema, table) in tables {
        // Safe: names come from the catalog and are quoted.
        let sql = format!(
            "SELECT coalesce(string_agg(row_to_json(t)::text, E'\\n'), '')
             FROM \"{}\".\"{}\" t",
            schema.replace('"', "\"\""),
            table.replace('"', "\"\"")
        );
        let rows: String = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(db)
            .await
            .unwrap();
        out.push_str(&format!("-- {schema}.{table}\n{rows}\n"));
    }
    out
}

// ---- Searching every encoding -------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.len();
        let v = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..=n {
            out.push(B64[((v >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// The forms `bytes` could take when stored or logged: as text, hex either
/// case, and the part of base64 and base64url that doesn't depend on what
/// surrounds it, at each of the three alignments.
fn needles(bytes: &[u8]) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Ok(text) = std::str::from_utf8(bytes) {
        out.push(("text", text.to_owned()));
    }
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    out.push(("hex", hex.to_uppercase()));
    out.push(("hex", hex));
    for k in 0..3 {
        let mut shifted = vec![0u8; k];
        shifted.extend_from_slice(bytes);
        let s = base64(&shifted);
        let start = (8 * k).div_ceil(6);
        let end = (8 * (k + bytes.len())) / 6;
        let stable = &s[start..end];
        out.push(("base64", stable.to_owned()));
        out.push(("base64url", stable.replace('+', "-").replace('/', "_")));
    }
    out.push((
        "decimal bytes",
        bytes
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(","),
    ));
    out
}

/// Where any of `secrets` shows up in `haystacks`.
fn leaks(secrets: &[Secret], haystacks: &[(&str, &str)]) -> Vec<String> {
    let mut found = Vec::new();
    for s in secrets {
        for (form, needle) in needles(&s.bytes) {
            for (place, hay) in haystacks {
                if hay.contains(&needle) {
                    found.push(format!("{} ({form}) in {place}", s.what));
                }
            }
        }
    }
    found
}

fn secret(what: &str, bytes: impl Into<Vec<u8>>) -> Secret {
    Secret {
        what: what.into(),
        bytes: bytes.into(),
    }
}

// ---- Setting up writers ---------------------------------------------------------------

async fn writer(client: &Client, handle: &str) -> (Persona, Store) {
    let p = client
        .create_persona(&CreatePersona {
            handle: handle.into(),
            display_name: None,
            summary: None,
        })
        .await
        .unwrap();
    let store = Store::in_memory();
    keys::init(&store, client, &p, false).await.unwrap();
    keys::publish(&store, client, &p, 5).await.unwrap();
    (p, store)
}

async fn signed_in(server: &TestServer, name: &str) -> Client {
    let c = server.client();
    let token = c
        .login(&Credentials {
            username: name.into(),
            password: TEST_PASSWORD.into(),
        })
        .await
        .unwrap();
    c.with_token(&token.token)
}

async fn linked(server: &TestServer, name: &str, existing: (&Store, &Client)) -> (Store, Client) {
    let store = Store::in_memory();
    let client = signed_in(server, name).await;
    let pending = devices::start_link(&store, &client).await.unwrap();
    devices::approve(existing.0, existing.1, &pending.code.parse().unwrap())
        .await
        .unwrap();
    devices::wait_for_link(
        &store,
        &client,
        &pending.link_id,
        Duration::from_millis(50),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    devices::finish_link(&store, &client).await.unwrap();
    (store, client)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_server_never_holds_plaintext_or_private_keys() {
    let logs = capture_logs();
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let db = server.db().await.unwrap();
    let canary = format!("CANARY{}", Uuid::new_v4().simple());
    // Channel names are lowercase slugs: a canary of their own.
    let channel = format!("c-{}", &Uuid::new_v4().simple().to_string()[..28]);
    let public = format!("PUBLIC{}", Uuid::new_v4().simple());
    let mut dumps: Vec<(String, String)> = Vec::new();

    // Two writers; the prompt is public by design.
    let alice_c = server.account("alice").await.unwrap();
    let (fox, alice) = writer(&alice_c, "ink_fox").await;
    let bob_c = server.account("bob").await.unwrap();
    let (wren, bob) = writer(&bob_c, "quiet_wren").await;
    let prompt = alice_c
        .create_prompt(&CreatePrompt {
            persona: fox.id,
            title: "The lighthouse ledger".into(),
            body: format!("A keeper finds a ledger. {public}"),
            maturity: Maturity::General,
            tags: Vec::new(),
            state: Some(PromptState::Open),
        })
        .await
        .unwrap();

    // The request and its pitch.
    let sent = requests::send(
        &bob,
        &bob_c,
        &wren,
        prompt.id,
        &format!("Pitch: {canary}"),
        None,
    )
    .await
    .unwrap();
    requests::inbox(&alice, &alice_c, prompt.id, false)
        .await
        .unwrap();
    let id = requests::accept(&alice, &alice_c, sent.request.id)
        .await
        .unwrap();
    for (s, c) in [(&alice, &alice_c), (&bob, &bob_c)] {
        assert!(sessions::sync(s, c).await.unwrap().problems.is_empty());
    }

    // Everything a writer writes.
    let first = sessions::write(&alice, &alice_c, &id, &format!("Story: {canary}"), None)
        .await
        .unwrap();
    sessions::name_channel(
        &alice,
        &alice_c,
        &id,
        &channel,
        Some(&format!("About: {canary}")),
    )
    .await
    .unwrap();
    let file_contents = format!("File contents: {canary}").repeat(50).into_bytes();
    let filed = sessions::post(
        &bob,
        &bob_c,
        &id,
        Draft {
            text: &format!("Planning: {canary}"),
            channel: Some(&channel),
            reply_to: Some(first),
            files: vec![(format!("{canary}.txt"), file_contents)],
        },
    )
    .await
    .unwrap();
    sessions::sync(&alice, &alice_c).await.unwrap();
    sessions::edit(&bob, &bob_c, &id, filed, &format!("Edited: {canary}"))
        .await
        .unwrap();
    let gone = sessions::write(&alice, &alice_c, &id, &format!("Deleted: {canary}"), None)
        .await
        .unwrap();
    sessions::delete(&alice, &alice_c, &id, gone).await.unwrap();
    for (s, c) in [(&alice, &alice_c), (&bob, &bob_c)] {
        sessions::sync(s, c).await.unwrap();
    }
    dumps.push(("the story written".into(), dump(&db).await));

    // A second device for Bob, and its history by bundle: dumped while the
    // bundle's blobs are on the server.
    let (phone, phone_c) = linked(&server, "bob", (&bob, &bob_c)).await;
    sessions::sync(&bob, &bob_c).await.unwrap();
    assert_eq!(
        devices::maintain(&phone, &phone_c)
            .await
            .unwrap()
            .history
            .asked,
        1
    );
    assert_eq!(
        devices::maintain(&bob, &bob_c)
            .await
            .unwrap()
            .history
            .answered,
        1
    );
    dumps.push(("a history bundle waiting".into(), dump(&db).await));
    devices::maintain(&phone, &phone_c).await.unwrap();
    let on_phone = sessions::read(&phone, &phone_c, &id).await.unwrap();
    assert!(
        on_phone
            .messages
            .iter()
            .any(|m| m.content.contains(&canary)),
        "the phone holds the story"
    );

    // A backup, restored on a new device that rejoins through Bob.
    let code = backup::create(&alice, &alice_c, false)
        .await
        .unwrap()
        .code
        .unwrap()
        .join(" ");
    dumps.push(("a backup stored".into(), dump(&db).await));
    let restored = Store::in_memory();
    let restored_c = signed_in(&server, "alice").await;
    backup::restore(&restored, &restored_c, &code)
        .await
        .unwrap();
    sessions::sync(&bob, &bob_c).await.unwrap();
    sessions::sync(&restored, &restored_c).await.unwrap();
    sessions::write(
        &restored,
        &restored_c,
        &id,
        &format!("Restored: {canary}"),
        None,
    )
    .await
    .unwrap();
    for (s, c) in [(&bob, &bob_c), (&phone, &phone_c)] {
        sessions::sync(s, c).await.unwrap();
    }
    let r = sessions::read(&bob, &bob_c, &id).await.unwrap();
    assert!(
        r.messages
            .iter()
            .any(|m| m.content.starts_with("Restored:"))
    );
    dumps.push(("the end".into(), dump(&db).await));

    // What must never reach the server: the canary in every form, and every
    // secret the clients hold.
    let mut secrets = vec![
        secret("the canary", canary.as_bytes()),
        secret("the channel name", channel.as_bytes()),
    ];
    for (who, store) in [
        ("Alice's first device", &alice),
        ("Alice's restored device", &restored),
        ("Bob's laptop", &bob),
        ("Bob's phone", &phone),
    ] {
        let held = testing::secrets(store).unwrap();
        let mls = held
            .iter()
            .filter(|s| s.what.starts_with("OpenMLS"))
            .count();
        assert!(mls > 0, "{who}: OpenMLS secrets found to check");
        assert!(held.iter().any(|s| s.what.starts_with("identity key")));
        assert!(held.iter().any(|s| s.what.starts_with("device key")));
        secrets.extend(held.into_iter().map(|mut s| {
            s.what = format!("{who}: {}", s.what);
            s
        }));
    }
    assert!(
        secrets.iter().any(|s| s.what.contains("recovery code")),
        "the recovery code is checked"
    );
    for m in &r.messages {
        for a in &m.attachments {
            secrets.push(secret(
                "an attachment's key",
                quill_proto::keys::unb64(&a.key).unwrap(),
            ));
            secrets.push(secret(
                "an attachment's blob token",
                quill_proto::keys::unb64(&a.blob_token).unwrap(),
            ));
        }
    }
    assert!(secrets.iter().any(|s| s.what == "an attachment's key"));

    let logs = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    assert!(
        logs.contains("quill_server") || logs.contains("sqlx"),
        "the server's logs were captured"
    );
    let mut places: Vec<(String, &str)> = dumps
        .iter()
        .map(|(when, d)| (format!("the database dump after {when}"), d.as_str()))
        .collect();
    places.push(("the server's logs".into(), logs.as_str()));
    let haystacks: Vec<(&str, &str)> = places.iter().map(|(p, h)| (p.as_str(), *h)).collect();

    // Positive controls: the search finds what is meant to be there.
    let last = &dumps.last().unwrap().1;
    assert!(last.contains(&public), "the public prompt is in the dump");
    let fox_key = keys::load_identity(&alice, fox.id)
        .unwrap()
        .unwrap()
        .public();
    let found = leaks(&[secret("the public identity key", fox_key)], &haystacks);
    assert!(
        found
            .iter()
            .any(|f| f.contains("(hex) in the database dump")),
        "the public identity key is found as hex: {found:?}"
    );
    for (bytes, around) in [
        (b"abcdefghijklmnop".as_slice(), 0usize),
        (b"0123456789abcdef", 1),
        (b"QRSTUVWXYZ012345", 2),
    ] {
        let mut wrapped = vec![b'x'; around];
        wrapped.extend_from_slice(bytes);
        wrapped.extend_from_slice(b"yz");
        let encoded = base64(&wrapped);
        let forms = needles(bytes);
        assert!(
            forms
                .iter()
                .any(|(f, n)| *f == "base64" && encoded.contains(n.as_str())),
            "base64 at alignment {around} is searched"
        );
    }

    let found = leaks(&secrets, &haystacks);
    assert!(
        found.is_empty(),
        "plaintext or secrets reached the server:\n{}",
        found.join("\n")
    );
    // Negative control: a writer who pastes the canary somewhere public,
    // even base64url-encoded at an odd alignment, is caught.
    let mut pasted = b"x".to_vec();
    pasted.extend_from_slice(canary.as_bytes());
    let pasted = quill_proto::keys::b64(&pasted);
    bob_c
        .update_persona(
            wren.id,
            &quill_proto::api::UpdatePersona {
                display_name: None,
                summary: Some(format!("Notes: {pasted}")),
            },
        )
        .await
        .unwrap();
    let after = dump(&db).await;
    let caught = leaks(&secrets[..1], &[("the planted dump", after.as_str())]);
    assert!(
        caught.iter().any(|c| c.contains("(base64url)")),
        "a planted canary is caught: {caught:?}"
    );

    eprintln!(
        "checked {} secrets in {} database dumps ({} bytes) and {} bytes of server logs",
        secrets.len(),
        dumps.len(),
        dumps.iter().map(|(_, d)| d.len()).sum::<usize>(),
        logs.len()
    );
    server.stop().await.unwrap();
}
