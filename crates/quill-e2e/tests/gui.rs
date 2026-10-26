//! The desktop app's native backend (3b) against a real server, without a
//! window: what the first run, the frame and the settings screens rely on.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{cell::RefCell, sync::Arc};

use quill_core::store::vault::MemoryVault;
use quill_e2e::TestServer;
use quill_gui::{NativeBackend, settings::Paths};
use quill_ui::backend::{Backend, Keys, Problem, Standing, Step};

type Vault = Option<Arc<dyn quill_core::store::vault::Vault>>;

/// A stand-in for the OS keychain, kept across app restarts in a test.
fn keychain() -> Vault {
    Some(Arc::new(MemoryVault::default()))
}

fn backend(dir: &std::path::Path, vault: &Vault) -> NativeBackend {
    NativeBackend::new(
        Paths::in_dir(dir),
        vault.clone(),
        tokio::runtime::Handle::current(),
    )
    .unwrap()
}

fn personas(s: &Standing) -> Vec<(String, Keys)> {
    match s {
        Standing::SignedIn { personas, .. } => personas
            .iter()
            .map(|p| (p.persona.handle.clone(), p.keys))
            .collect(),
        other => panic!("not signed in: {other:?}"),
    }
}

const PASSWORD: &str = "correct horse battery";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_writer_gets_from_nothing_to_a_ready_persona() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let vault = keychain();
    let app = backend(dir.path(), &vault);
    assert!(app.servers().is_empty());

    let info = app.check_server(&server.url()).await.unwrap();
    assert!(info.registration_open);
    let added = app.add_server(&server.url(), None).await.unwrap();
    assert_eq!(app.current(), Some(added.id));
    assert_eq!(app.standing().await.unwrap(), Standing::SignedOut);

    let standing = app.register("ink", PASSWORD).await.unwrap();
    assert!(
        personas(&standing).is_empty(),
        "skipping the persona is fine"
    );

    let fox = app.create_persona("ink_fox", "Ink Fox").await.unwrap();
    assert_eq!(
        personas(&app.standing().await.unwrap()),
        vec![("ink_fox".to_owned(), Keys::ToSetUp)]
    );
    let steps = RefCell::new(Vec::new());
    app.prepare(fox.id, &|s| steps.borrow_mut().push(s))
        .await
        .unwrap();
    assert_eq!(*steps.borrow(), Step::ALL.to_vec());
    assert_eq!(
        personas(&app.standing().await.unwrap()),
        vec![("ink_fox".to_owned(), Keys::Ready)]
    );
    // Doing it again is harmless.
    app.prepare(fox.id, &|_| {}).await.unwrap();
    assert_eq!(app.upkeep().await.unwrap(), None);

    // A taken handle is refused in words.
    let taken = app.create_persona("ink_fox", "").await.unwrap_err();
    assert!(matches!(taken, Problem::Refused(_)), "{taken:?}");

    // The app opens the same server again next time, from its settings.
    drop(app);
    let again = backend(dir.path(), &vault);
    assert_eq!(again.servers().len(), 1);
    let id = again.current().unwrap();
    let standing = again.open_server(id).await.unwrap();
    assert_eq!(
        personas(&standing),
        vec![("ink_fox".to_owned(), Keys::Ready)]
    );
    match standing {
        Standing::SignedIn { login, .. } => assert_eq!(login, "ink"),
        _ => unreachable!(),
    }
    // Advanced mode's details name the ids that nothing else shows.
    let details = again.details().await.unwrap();
    assert!(
        details
            .rows
            .iter()
            .any(|(k, v)| k == "@ink_fox id" && *v == fox.id.to_string())
    );
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signing_in_on_a_second_computer_finds_the_keys_elsewhere() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let first_dir = tempfile::tempdir().unwrap();
    let first = backend(first_dir.path(), &keychain());
    first.add_server(&server.url(), None).await.unwrap();
    first.register("ink", PASSWORD).await.unwrap();
    let fox = first.create_persona("ink_fox", "Ink Fox").await.unwrap();
    first.prepare(fox.id, &|_| {}).await.unwrap();

    let second_dir = tempfile::tempdir().unwrap();
    let second = backend(second_dir.path(), &keychain());
    second.add_server(&server.url(), None).await.unwrap();
    let wrong = second.sign_in("ink", "not the password").await.unwrap_err();
    assert!(matches!(wrong, Problem::Refused(_)), "{wrong:?}");
    let standing = second.sign_in("ink", PASSWORD).await.unwrap();
    assert_eq!(
        personas(&standing),
        vec![("ink_fox".to_owned(), Keys::Elsewhere)]
    );
    // Getting it ready here refuses, pointing at linking.
    let refused = second.prepare(fox.id, &|_| {}).await.unwrap_err();
    assert!(matches!(refused, Problem::Refused(_)), "{refused:?}");

    second.sign_out().await.unwrap();
    assert_eq!(second.standing().await.unwrap(), Standing::SignedOut);
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn servers_are_added_switched_and_removed() {
    let (Some(a), Some(b)) = (
        TestServer::start().await.unwrap(),
        TestServer::start_with(|c| c.registration_open = false)
            .await
            .unwrap(),
    ) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let vault = keychain();
    let app = backend(dir.path(), &vault);
    let first = app.add_server(&a.url(), None).await.unwrap();
    app.register("ink", PASSWORD).await.unwrap();
    app.create_persona("ink_fox", "").await.unwrap();

    // The second server takes no new logins: it says so before asking.
    assert!(!app.check_server(&b.url()).await.unwrap().registration_open);
    let second = app.add_server(&b.url(), None).await.unwrap();
    assert_eq!(app.current(), Some(second.id));
    let closed = app.register("ink", PASSWORD).await.unwrap_err();
    assert!(matches!(closed, Problem::Refused(_)), "{closed:?}");
    assert_eq!(app.standing().await.unwrap(), Standing::SignedOut);
    // The same server can't be added twice.
    assert!(app.add_server(&b.url(), None).await.is_err());

    // Switching back finds the first login as it was.
    let standing = app.open_server(first.id).await.unwrap();
    assert_eq!(personas(&standing).len(), 1);
    assert_eq!(app.current(), Some(first.id));

    // Moving the stores' folder keeps both, and the open one stays open.
    let elsewhere = tempfile::tempdir().unwrap();
    let to = elsewhere.path().join("quill-stores");
    app.move_stores(to.to_str().unwrap()).await.unwrap();
    assert_eq!(app.app_settings().stores_folder, to.display().to_string());
    assert!(to.join(format!("{}.quill", second.id)).exists());
    assert_eq!(personas(&app.standing().await.unwrap()).len(), 1);
    assert_eq!(
        app.open_server(second.id).await.unwrap(),
        Standing::SignedOut
    );

    app.remove_server(second.id).await.unwrap();
    assert_eq!(app.servers().len(), 1);
    assert!(!to.join(format!("{}.quill", second.id)).exists());
    a.stop().await.unwrap();
    b.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_keychain_the_store_takes_a_passphrase() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let app = backend(dir.path(), &None);
    assert_eq!(
        app.add_server(&server.url(), None).await.unwrap_err(),
        Problem::NeedsPassphrase
    );
    let added = app
        .add_server(&server.url(), Some("a long store passphrase"))
        .await
        .unwrap();
    app.register("ink", PASSWORD).await.unwrap();
    drop(app);

    let again = backend(dir.path(), &None);
    assert_eq!(again.open_server(added.id).await.unwrap(), Standing::Locked);
    assert_eq!(again.standing().await.unwrap(), Standing::Locked);
    let wrong = again.unlock("not the passphrase").await.unwrap_err();
    assert!(matches!(wrong, Problem::Refused(_)), "{wrong:?}");
    let standing = again.unlock("a long store passphrase").await.unwrap();
    assert!(personas(&standing).is_empty());
    server.stop().await.unwrap();
}

#[tokio::test]
async fn an_unreachable_or_mistyped_server_is_said_plainly() {
    let dir = tempfile::tempdir().unwrap();
    let vault = keychain();
    let app = backend(dir.path(), &vault);
    // Nothing listens on port 9 here.
    let down = app.check_server("http://127.0.0.1:9").await.unwrap_err();
    assert!(matches!(down, Problem::Unreachable(_)), "{down:?}");
    let insecure = app
        .check_server("http://writers.example")
        .await
        .unwrap_err();
    assert!(matches!(insecure, Problem::Refused(_)), "{insecure:?}");
    assert!(matches!(
        app.check_server("  ").await.unwrap_err(),
        Problem::Refused(_)
    ));
}
