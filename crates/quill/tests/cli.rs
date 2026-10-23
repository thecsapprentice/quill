//! The CLI end to end: its login lives in the encrypted local store.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{path::Path, process::Command};

use quill_e2e::TestServer;

const PASSPHRASE: &str = "a long enough passphrase";

/// Runs `quill` with a store in `dir`; returns (success, stdout, stderr).
/// Blocks its thread, so the test runtime needs another for the server.
fn quill(dir: &Path, passphrase: &str, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_quill"))
        .args(args)
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
