//! The desktop app worked as a person would (by labels, through the same
//! events the window sends), on the native backend against a real server.
//! `quill-ui`'s `tests/flows.rs` covers far more against a simulated
//! server; these check the simulation holds for the paths that matter.

// Test code: panicking on failure is intended.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{rc::Rc, sync::Arc};

use quill_core::store::vault::{MemoryVault, Vault};
use quill_e2e::TestServer;
use quill_gui::{NativeBackend, settings::Paths};
use quill_proto::{
    board::UpdateServerSettings,
    rules::{RulesAcceptance, UpdateRules},
};
use quill_ui::testing::Driver;

const PASSWORD: &str = "correct horse battery";

fn app(dir: &std::path::Path, vault: &Arc<MemoryVault>) -> Driver {
    let vault: Arc<dyn Vault> = vault.clone();
    let backend = NativeBackend::new(
        Paths::in_dir(dir),
        Some(vault),
        tokio::runtime::Handle::current(),
    )
    .unwrap();
    Driver::new(Rc::new(backend))
}

/// The server's address as a writer types it.
fn typed(server: &TestServer) -> String {
    server.url().trim_start_matches("http://").to_owned()
}

async fn to_sign_up(d: &mut Driver, server: &TestServer) {
    d.type_into("server-address", &typed(server)).await;
    d.wait_for_text("Takes new logins").await;
    d.click("Continue").await;
    d.wait_for_text("Make a login on").await;
}

async fn set_rules(server: &TestServer, body: &str, acceptance: RulesAcceptance) {
    let admin = server.admin_account("boss").await.unwrap();
    admin
        .admin_update_rules(&UpdateRules {
            body: body.into(),
            minor: false,
        })
        .await
        .unwrap();
    admin
        .admin_update_settings(&UpdateServerSettings {
            rules_acceptance: Some(acceptance),
            ..UpdateServerSettings::default()
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_run_against_a_real_server() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(MemoryVault::default());
    let mut d = app(dir.path(), &vault);
    to_sign_up(&mut d, &server).await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    d.type_into("persona-handle", "ink_fox").await;
    d.click("Make this persona").await;
    d.wait_for_text("Hello, @ink_fox").await;
    d.wait_for_text("Connected to").await;

    // Opening again goes straight to the frame.
    drop(d);
    let mut d = app(dir.path(), &vault);
    d.wait_for_text("Hello, @ink_fox").await;
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rules_required_after_sign_up_opened_against_a_real_server() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(MemoryVault::default());
    let mut d = app(dir.path(), &vault);
    to_sign_up(&mut d, &server).await;
    // The administrator requires the rules while the screen is open.
    set_rules(&server, "# Be kind\n\nNo spam.", RulesAcceptance::OnJoin).await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Make my login").await;
    d.wait_for_text("I've read and accept the server rules")
        .await;
    assert!(d.button_disabled("Make my login"));
    d.set_checked("accept-rules", true).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rules_on_change_are_accepted_at_sign_up_against_a_real_server() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    set_rules(&server, "# Be kind", RulesAcceptance::OnChange).await;
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(MemoryVault::default());
    let mut d = app(dir.path(), &vault);
    to_sign_up(&mut d, &server).await;
    d.wait_for_text("I've read and accept the server rules")
        .await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.set_checked("accept-rules", true).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    d.click("Not now").await;
    d.wait_for_text("Connected to").await;
    // The server recorded it: no "rules have changed" right after joining.
    d.settle().await;
    assert!(!d.has_text("The server rules have changed"));
    server.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_add_a_server_against_a_real_server() {
    let Some(server) = TestServer::start().await.unwrap() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let vault = Arc::new(MemoryVault::default());
    let mut d = app(dir.path(), &vault);
    to_sign_up(&mut d, &server).await;
    d.type_into("login-name", "ink").await;
    d.type_into("login-password", PASSWORD).await;
    d.click("Make my login").await;
    d.wait_for_text("Make your first persona").await;
    d.click("Not now").await;
    d.wait_for_text("Connected to").await;
    d.choose("server-switch", "new").await;
    d.wait_for_text("Add a server").await;
    d.click("Cancel").await;
    d.wait_for_text("Connected to").await;
    server.stop().await.unwrap();
}
