//! The CLI end to end: its login lives in the encrypted local store.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, process::Command};

use quill_e2e::TestServer;

const PASSPHRASE: &str = "a long enough passphrase";

/// Runs `quill` with a store in `dir`; returns (success, stdout, stderr).
/// Blocks its thread, so the test runtime needs another for the server.
fn quill(dir: &Path, passphrase: &str, args: &[&str]) -> (bool, String, String) {
    quill_with(dir, passphrase, &[], args)
}

fn quill_with(
    dir: &Path,
    passphrase: &str,
    env: &[(&str, &str)],
    args: &[&str],
) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_quill"))
        .args(args)
        .envs(env.iter().copied())
        .env("QUILL_STORE", dir.join("store.quill"))
        .env("QUILL_PROFILE", dir.join("profile.json"))
        .env("QUILL_STORE_PASSPHRASE", passphrase)
        .env("QUILL_PASSWORD", "a writer's password")
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Links the device whose store is in `new` (signed in already) from the
/// one in `existing`, as a user would: `device link` waits, `device
/// approve` answers, the words match, the user says yes. Returns what
/// `device link` printed after that.
fn link(existing: &Path, new: &Path) -> String {
    use std::io::{BufRead, BufReader, Write};

    // The new device waits for approval, showing its code.
    let mut link = std::process::Command::new(env!("CARGO_BIN_EXE_quill"))
        .args(["device", "link"])
        .env("QUILL_STORE", new.join("store.quill"))
        .env("QUILL_PROFILE", new.join("profile.json"))
        .env("QUILL_STORE_PASSPHRASE", PASSPHRASE)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = BufReader::new(link.stdout.take().unwrap());
    let code = loop {
        let mut line = String::new();
        assert!(out.read_line(&mut line).unwrap() > 0, "link exited early");
        if let Some(code) = line.trim().strip_prefix("quill device approve ") {
            break code.to_owned();
        }
    };

    // The existing device approves and shows the words.
    let (ok, shown, err) = quill(existing, PASSPHRASE, &["device", "approve", &code]);
    assert!(ok, "{err}");
    let existing_words = shown
        .lines()
        .find_map(|l| l.strip_prefix("It should now show: "))
        .unwrap()
        .to_owned();

    // The new device shows the same words; the user says yes.
    let new_words = loop {
        let mut line = String::new();
        assert!(out.read_line(&mut line).unwrap() > 0, "link exited early");
        if let Some(w) = line.trim().strip_prefix("Approved. This device shows: ") {
            break w.to_owned();
        }
    };
    assert_eq!(new_words, existing_words);
    link.stdin.take().unwrap().write_all(b"y\n").unwrap();
    let mut rest = String::new();
    std::io::Read::read_to_string(&mut out, &mut rest).unwrap();
    assert!(link.wait().unwrap().success(), "{rest}");
    rest
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cli_keeps_its_login_in_the_encrypted_store() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let url = server.url();
    let run = |args: &[&str]| quill(dir.path(), PASSPHRASE, args);

    // An old plain profile is taken over by the new store, then deleted.
    std::fs::write(
        dir.path().join("profile.json"),
        format!(r#"{{"server":"{url}"}}"#),
    )
    .unwrap();
    let (ok, out, err) = run(&["server"]);
    assert!(ok, "{err}");
    assert_eq!(out.trim(), url);
    assert!(err.contains("Created your encrypted local store"), "{err}");
    assert!(!dir.path().join("profile.json").exists());

    let (ok, _, err) = run(&["register", "cli_writer"]);
    assert!(ok, "{err}");
    let (ok, _, err) = run(&["persona", "new", "cli_pen"]);
    assert!(ok, "{err}");
    let (ok, out, _) = run(&["--json", "persona", "list"]);
    assert!(ok);
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list[0]["handle"], "cli_pen");

    // Nothing readable on disk: not the server, the login or the token.
    let raw = std::fs::read(dir.path().join("store.quill")).unwrap();
    for needle in ["127.0.0.1", "qt_", "cli_writer"] {
        assert!(
            !raw.windows(needle.len()).any(|w| w == needle.as_bytes()),
            "{needle} in the store file"
        );
    }

    let (ok, out, _) = run(&["--json", "store", "status"]);
    assert!(ok);
    let status: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(status["protectedBy"], "passphrase");

    // A wrong passphrase gets nothing; changing it locks out the old one.
    let (ok, _, err) = quill(dir.path(), "not the passphrase", &["persona", "list"]);
    assert!(!ok && err.contains("wrong passphrase"), "{err}");
    // The new passphrase comes from its own variable when not typed.
    let new = "an entirely different passphrase";
    let out = Command::new(env!("CARGO_BIN_EXE_quill"))
        .args(["store", "passphrase"])
        .env("QUILL_STORE", dir.path().join("store.quill"))
        .env("QUILL_PROFILE", dir.path().join("profile.json"))
        .env("QUILL_STORE_PASSPHRASE", PASSPHRASE)
        .env("QUILL_STORE_NEW_PASSPHRASE", new)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (ok, _, _) = run(&["persona", "list"]);
    assert!(!ok);
    let (ok, _, err) = quill(dir.path(), new, &["persona", "list"]);
    assert!(ok, "{err}");

    // Short passphrases are refused for new stores.
    let other = tempfile::tempdir().unwrap();
    let (ok, _, err) = quill(other.path(), "short", &["server"]);
    assert!(!ok && err.contains("at least 12"), "{err}");
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cli_publishes_and_checks_persona_keys() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let alice = tempfile::tempdir().unwrap();
    let bob = tempfile::tempdir().unwrap();
    let url = server.url();
    let as_alice = |args: &[&str]| quill(alice.path(), PASSPHRASE, args);
    let as_bob = |args: &[&str]| quill(bob.path(), PASSPHRASE, args);
    assert!(as_alice(&["server", &url]).0);
    assert!(as_bob(&["server", &url]).0);
    assert!(as_alice(&["register", "cli_alice"]).0);
    assert!(as_alice(&["persona", "new", "ink_fox"]).0);

    let (ok, out, err) = as_alice(&["--json", "keys", "init", "--as", "@ink_fox"]);
    assert!(ok, "{err}");
    let init: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(init["identityKeyVersion"], 1);
    let fingerprint = init["fingerprint"].as_str().unwrap().to_owned();
    assert_eq!(init["keyPackages"], 20);
    let (ok, out, _) = as_alice(&["keys", "status", "--as", "ink_fox"]);
    assert!(ok && out.contains("KeyPackages waiting: 20"), "{out}");
    let device = init["device"].as_str().unwrap().to_owned();

    // Bob (not even signed in) checks and pins Alice's persona's key.
    let (ok, out, err) = as_bob(&["keys", "show", "ink_fox"]);
    assert!(ok, "{err}");
    assert!(out.contains(&fingerprint), "{out}");
    assert!(out.contains("first time seen: pinned"), "{out}");
    assert!(out.contains(&format!("{device}  valid")), "{out}");
    let (ok, out, _) = as_bob(&["--json", "keys", "show", "ink_fox"]);
    assert!(ok);
    let shown: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(shown["pin"], "matches");

    // Bob signs up and claims one of Alice's packages, checking it.
    assert!(as_bob(&["register", "cli_bob"]).0);
    let (ok, out, err) = as_bob(&["keys", "claim", "ink_fox"]);
    assert!(ok, "{err}");
    assert!(
        out.contains(&format!("{device}  KeyPackage checks out")),
        "{out}"
    );
    let (_, out, _) = as_alice(&["keys", "status", "--as", "ink_fox"]);
    assert!(out.contains("KeyPackages waiting: 19"), "{out}");

    let (ok, out, _) = as_alice(&["device", "list", "--as", "ink_fox"]);
    assert!(ok);
    assert!(out.contains("(this device)"), "{out}");

    // Replacing the key from a new store: refused unless asked for, and
    // then Bob is warned until he accepts the change.
    let other = tempfile::tempdir().unwrap();
    let as_other = |args: &[&str]| quill(other.path(), PASSPHRASE, args);
    assert!(as_other(&["server", &url]).0);
    assert!(as_other(&["login", "cli_alice"]).0);
    let (ok, _, err) = as_other(&["keys", "init", "--as", "ink_fox"]);
    assert!(!ok && err.contains("--new-identity"), "{err}");
    assert!(as_other(&["keys", "init", "--as", "ink_fox", "--new-identity"]).0);
    let (ok, out, err) = as_bob(&["keys", "show", "ink_fox"]);
    assert!(
        !ok && out.contains("CHANGED") && err.contains("changed"),
        "{out}{err}"
    );
    assert!(as_bob(&["keys", "show", "ink_fox", "--accept-change"]).0);
    assert!(as_bob(&["keys", "show", "ink_fox"]).0);

    // The new store revokes the old device.
    let (ok, out, err) = as_other(&[
        "device", "revoke", &device, "--as", "ink_fox", "--reason", "lost",
    ]);
    assert!(ok, "{err}");
    assert!(out.contains("revoked"), "{out}");
    let (_, out, _) = as_bob(&["keys", "show", "ink_fox"]);
    assert!(out.contains(&format!("{device}  revoked (lost)")), "{out}");
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cli_links_a_second_computer() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let laptop = tempfile::tempdir().unwrap();
    let desktop = tempfile::tempdir().unwrap();
    let url = server.url();
    let on_laptop = |args: &[&str]| quill(laptop.path(), PASSPHRASE, args);
    let on_desktop = |args: &[&str]| quill(desktop.path(), PASSPHRASE, args);
    assert!(on_laptop(&["server", &url]).0);
    assert!(on_laptop(&["register", "cli_alice"]).0);
    assert!(on_laptop(&["persona", "new", "ink_fox"]).0);
    assert!(on_laptop(&["keys", "init", "--as", "ink_fox"]).0);
    assert!(on_desktop(&["server", &url]).0);
    assert!(on_desktop(&["login", "cli_alice"]).0);

    let rest = link(laptop.path(), desktop.path());
    assert!(rest.contains("@ink_fox"), "{rest}");

    // Both computers list both devices.
    for run in [
        &on_laptop as &dyn Fn(&[&str]) -> (bool, String, String),
        &on_desktop,
    ] {
        let (ok, out, err) = run(&["--json", "devices"]);
        assert!(ok, "{err}");
        let list: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2, "{out}");
    }
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cli_sends_reads_and_accepts_a_request() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let alice = tempfile::tempdir().unwrap();
    let bob = tempfile::tempdir().unwrap();
    let url = server.url();
    let as_alice = |args: &[&str]| quill(alice.path(), PASSPHRASE, args);
    let as_bob = |args: &[&str]| quill(bob.path(), PASSPHRASE, args);
    for (who, name, handle) in [
        (&as_alice as &dyn Fn(&[&str]) -> _, "cli_alice", "ink_fox"),
        (&as_bob, "cli_bob", "quiet_wren"),
    ] {
        assert!(who(&["server", &url]).0);
        assert!(who(&["register", name]).0);
        assert!(who(&["persona", "new", handle]).0);
        let (ok, _, err) = who(&["keys", "init", "--as", handle]);
        assert!(ok, "{err}");
    }
    let (ok, out, err) = as_alice(&[
        "--json",
        "prompt",
        "post",
        "--as",
        "ink_fox",
        "--title",
        "The lighthouse ledger",
        "--body",
        "A keeper finds a ledger.",
        "--maturity",
        "general",
    ]);
    assert!(ok, "{err}");
    let prompt: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(prompt["acceptingSessions"], true);
    let prompt = prompt["id"].as_str().unwrap().to_owned();

    let (ok, out, err) = as_bob(&[
        "--json",
        "request",
        "send",
        &prompt,
        "--as",
        "quiet_wren",
        "--pitch",
        "I'd write the keeper.",
        "--days",
        "7",
    ]);
    assert!(ok, "{err}");
    let sent: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(sent["authorDevices"], 1);
    let id = sent["id"].as_str().unwrap().to_owned();
    let (ok, out, _) = as_bob(&["request", "list"]);
    assert!(ok && out.contains(&format!("{id}  waiting")), "{out}");
    // Only one waiting request per prompt.
    let (ok, _, err) = as_bob(&[
        "request",
        "send",
        &prompt,
        "--as",
        "quiet_wren",
        "--pitch",
        "Again?",
    ]);
    assert!(
        !ok && err.contains("already have a request waiting"),
        "{err}"
    );

    // Accepting needs the pitch read on this device first.
    let (ok, _, err) = as_alice(&["request", "accept", &id]);
    assert!(!ok && err.contains("read the request"), "{err}");
    let (ok, out, err) = as_alice(&["request", "inbox", &prompt]);
    assert!(ok, "{err}");
    assert!(out.contains("from @quiet_wren"), "{out}");
    assert!(out.contains("    I'd write the keeper."), "{out}");
    let (ok, out, err) = as_alice(&["request", "accept", &id]);
    assert!(ok && out.contains("accepted"), "{out}{err}");
    let (_, out, _) = as_bob(&["request", "list"]);
    assert!(out.contains(&format!("{id}  session")), "{out}");

    // Writing together (2f).
    let (ok, out, err) = as_bob(&["--json", "sessions"]);
    assert!(ok, "{err}");
    let list: serde_json::Value = serde_json::from_str(&out).unwrap();
    let session = list[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(list[0]["partner"], "ink_fox");
    let short = &session[..10];
    let (ok, out, err) = as_bob(&[
        "session",
        "write",
        short,
        "--text",
        "The tide came in early.",
    ]);
    assert!(ok && out.contains("sent"), "{out}{err}");
    let (ok, out, err) = as_alice(&["session", "read", short]);
    assert!(ok, "{err}");
    assert!(
        out.contains("@quiet_wren:\n    I'd write the keeper."),
        "{out}"
    );
    assert!(
        out.contains("[story] @quiet_wren:\n    The tide came in early."),
        "{out}"
    );
    // Attachments and storage (2i).
    let file = bob.path().join("map.txt");
    std::fs::write(&file, b"North is up.").unwrap();
    let (ok, out, err) = as_bob(&[
        "--json",
        "session",
        "write",
        short,
        "--text",
        "A map.",
        "--attach",
        file.to_str().unwrap(),
    ]);
    assert!(ok, "{err}");
    let map_seq = serde_json::from_str::<serde_json::Value>(&out).unwrap()["seq"].to_string();
    let (_, out, _) = as_alice(&["session", "read", short]);
    assert!(
        out.contains("[attachment] map.txt (12 bytes, text/plain)"),
        "{out}"
    );
    let saved = alice.path().join("saved");
    let (ok, _, err) = as_alice(&[
        "session",
        "save",
        short,
        &map_seq,
        "--to",
        saved.to_str().unwrap(),
    ]);
    assert!(ok, "{err}");
    assert_eq!(
        std::fs::read(saved.join("map.txt")).unwrap(),
        b"North is up."
    );
    let (ok, out, err) = as_bob(&["storage"]);
    assert!(ok, "{err}");
    assert!(out.contains("@quiet_wren with @ink_fox"), "{out}");
    assert!(
        out.contains("1 attachments (4.0 KiB, 4.0 KiB yours)"),
        "{out}"
    );

    // Channels (2h).
    let (ok, _, err) = as_alice(&[
        "session",
        "channel",
        "add",
        short,
        "world",
        "--about",
        "Places and people",
    ]);
    assert!(ok, "{err}");
    let (ok, out, err) = as_bob(&["session", "channels", short]);
    assert!(ok, "{err}");
    assert!(
        out.contains("world  0 notes, 0 new  Places and people"),
        "{out}"
    );
    let (ok, out, err) = as_bob(&[
        "--json",
        "session",
        "write",
        short,
        "--channel",
        "world",
        "--text",
        "The lighthouse.",
    ]);
    assert!(ok, "{err}");
    let seq = serde_json::from_str::<serde_json::Value>(&out).unwrap()["seq"].to_string();
    let (ok, _, err) = as_alice(&[
        "session",
        "write",
        short,
        "--reply-to",
        &seq,
        "--text",
        "Act one, then.",
    ]);
    assert!(ok, "{err}");
    let (_, out, _) = as_bob(&["session", "read", short, "--channel", "story"]);
    assert!(
        out.contains(&format!("(answers #{seq} in [world])")),
        "{out}"
    );
    assert!(!out.contains("The lighthouse."), "{out}");
    // Safety numbers and backups (2j).
    let (ok, out, err) = as_alice(&["--json", "session", "verify", short]);
    assert!(ok, "{err}");
    let alices: serde_json::Value = serde_json::from_str(&out).unwrap();
    let (ok, out, err) = as_bob(&["--json", "session", "verify", short, "--confirm"]);
    assert!(ok, "{err}");
    let bobs: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(alices["identity"], bobs["identity"]);
    assert_eq!(bobs["verified"], true);
    let (_, out, _) = as_bob(&["sessions"]);
    assert!(
        out.contains("@quiet_wren with @ink_fox (verified)"),
        "{out}"
    );
    // Bob links a second computer: his laptop adds it to the session, and
    // sends it what came before (2k).
    let desktop = tempfile::tempdir().unwrap();
    let on_desktop = |args: &[&str]| quill(desktop.path(), PASSPHRASE, args);
    assert!(on_desktop(&["server", &url]).0);
    assert!(on_desktop(&["login", "cli_bob"]).0);
    let rest = link(bob.path(), desktop.path());
    assert!(rest.contains("@quiet_wren"), "{rest}");
    assert!(as_bob(&["sessions"]).0);
    let (ok, out, err) = on_desktop(&["sessions"]);
    assert!(ok, "{err}");
    assert!(out.contains("earlier history still to come"), "{out}");
    let (_, _, err) = on_desktop(&["session", "read", short]);
    assert!(
        err.contains("doesn't have all of the session's earlier history"),
        "{err}"
    );
    assert!(as_bob(&["sessions"]).0);
    let (ok, out, err) = on_desktop(&["session", "read", short]);
    assert!(ok, "{err}");
    assert!(out.contains("The lighthouse."), "{out}");
    assert!(
        out.contains("you:"),
        "Bob's own notes are his here too: {out}"
    );
    assert!(!err.contains("earlier history"), "{err}");
    let (_, out, _) = on_desktop(&["sessions"]);
    assert!(!out.contains("earlier history still to come"), "{out}");

    let (ok, out, err) = as_alice(&["--json", "backup", "create"]);
    assert!(ok, "{err}");
    let code = serde_json::from_str::<serde_json::Value>(&out).unwrap()["recoveryCode"]
        .as_str()
        .unwrap()
        .to_owned();
    let restored = tempfile::tempdir().unwrap();
    let as_new = |args: &[&str]| {
        quill_with(
            restored.path(),
            PASSPHRASE,
            &[("QUILL_RECOVERY_CODE", &code)],
            args,
        )
    };
    assert!(as_new(&["server", &url]).0);
    assert!(as_new(&["login", "cli_alice"]).0);
    let (ok, out, err) = as_new(&["backup", "restore", "--yes"]);
    assert!(ok, "{err}");
    assert!(out.contains("restored: @ink_fox"), "{out}");
    assert!(out.contains("asked to rejoin 1 sessions"), "{out}");
    let (ok, out, err) = as_new(&["session", "read", short]);
    assert!(ok, "{err}");
    assert!(out.contains("The tide came in early."), "{out}");

    let (ok, out, err) = as_alice(&["session", "leave", short]);
    assert!(ok && out.contains("ended"), "{out}{err}");
    let (ok, _, err) = as_bob(&["session", "write", short, "--text", "Hello?"]);
    assert!(!ok && err.contains("ended"), "{err}");
    server.stop().await.unwrap();
}
