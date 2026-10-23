//! The encrypted local store: opening it, creating it on first use, and the
//! `quill store` commands.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use quill_core::{
    Profile, Store, StoreError,
    store::{KdfParams, KeySetup, KeySource, Unlock, vault::OsKeychain},
};
use zeroize::Zeroizing;

use crate::Ctx;

/// Environment variable read for the store passphrase in non-interactive use
/// (tests, scripts, machines without a keychain). Prefer the keychain.
pub const PASSPHRASE_ENV: &str = "QUILL_STORE_PASSPHRASE";

/// Environment variable read for the new passphrase by `quill store
/// passphrase`, when the current one is in [`PASSPHRASE_ENV`].
pub const NEW_PASSPHRASE_ENV: &str = "QUILL_STORE_NEW_PASSPHRASE";

/// Shortest passphrase accepted for a new store.
const MIN_PASSPHRASE: usize = 12;

#[derive(Subcommand)]
pub enum StoreCommand {
    /// Show where the store is and how it's protected.
    Status,
    /// Protect the store with a passphrase (new, or replacing the current
    /// passphrase or keychain key).
    Passphrase,
    /// Protect the store with a key kept in this computer's keychain.
    Keychain,
}

fn passphrase_from_env() -> Option<Zeroizing<String>> {
    std::env::var(PASSPHRASE_ENV).ok().map(Zeroizing::new)
}

fn prompt(label: &str) -> Result<Zeroizing<String>> {
    rpassword::prompt_password(label)
        .map(Zeroizing::new)
        .with_context(|| {
            format!("couldn't ask for the store passphrase (no terminal?); set {PASSPHRASE_ENV}")
        })
}

/// A new passphrase, from the environment or typed twice.
fn new_passphrase(changing: bool) -> Result<Zeroizing<String>> {
    let from_env = if changing {
        std::env::var(NEW_PASSPHRASE_ENV).ok().map(Zeroizing::new)
    } else {
        passphrase_from_env()
    };
    let p = match from_env {
        Some(p) => p,
        None => {
            let first = prompt("New store passphrase: ")?;
            let second = prompt("Repeat passphrase: ")?;
            if *first != *second {
                bail!("passphrases do not match");
            }
            first
        }
    };
    if p.chars().count() < MIN_PASSPHRASE {
        bail!("the passphrase must be at least {MIN_PASSPHRASE} characters");
    }
    Ok(p)
}

/// Opens the store, creating it on first use: with the keychain when this
/// computer has one, else with a passphrase.
pub fn open(path: &Path, legacy_profile: Option<PathBuf>) -> Result<Store> {
    if path.exists() {
        let store = match Store::key_source(path)? {
            KeySource::Keychain => {
                let keychain = OsKeychain::new()
                    .context("this store's key is in the system keychain, which isn't available")?;
                Store::open(path, Unlock::Keychain(&keychain))?
            }
            KeySource::Passphrase => {
                let p = match passphrase_from_env() {
                    Some(p) => p,
                    None => prompt("Store passphrase: ")?,
                };
                Store::open(path, Unlock::Passphrase(&p))?
            }
        };
        return Ok(store);
    }

    // A passphrase in the environment means the caller wants one; otherwise
    // prefer the keychain, so day-to-day use needs no typing.
    let store = match (passphrase_from_env(), OsKeychain::new()) {
        (None, Ok(keychain)) => match Store::create(path, KeySetup::Keychain(&keychain)) {
            Ok(s) => s,
            Err(StoreError::Keychain(why)) => {
                eprintln!("The system keychain refused the store key ({why}); using a passphrase.");
                create_with_passphrase(path)?
            }
            Err(e) => return Err(e.into()),
        },
        _ => create_with_passphrase(path)?,
    };
    eprintln!("Created your encrypted local store at {}.", path.display());
    if let Some(legacy) = legacy_profile {
        migrate(&store, &legacy)?;
    }
    Ok(store)
}

fn create_with_passphrase(path: &Path) -> Result<Store> {
    let p = new_passphrase(false)?;
    Ok(Store::create(
        path,
        KeySetup::Passphrase {
            passphrase: &p,
            kdf: KdfParams::DEFAULT,
        },
    )?)
}

/// Moves the server and login token from the old plain profile file into
/// the store, then deletes that file.
fn migrate(store: &Store, legacy: &Path) -> Result<()> {
    if !legacy.exists() {
        return Ok(());
    }
    let profile = Profile::load(legacy)?;
    if let Some(server) = &profile.server {
        store.set_server(server)?;
    }
    if let Some(token) = &profile.token {
        store.set_token(Some(token.as_str()))?;
    }
    std::fs::remove_file(legacy)
        .with_context(|| format!("remove the old profile file {}", legacy.display()))?;
    eprintln!(
        "Moved your server and login from {} into the store, and deleted that file.",
        legacy.display()
    );
    Ok(())
}

fn describe(source: Option<KeySource>) -> &'static str {
    match source {
        Some(KeySource::Keychain) => "system keychain",
        Some(KeySource::Passphrase) => "passphrase",
        None => "none (in memory)",
    }
}

pub fn command(ctx: &mut Ctx, cmd: StoreCommand) -> Result<()> {
    let before = ctx.store.status()?.key_source;
    match cmd {
        StoreCommand::Status => {
            let s = ctx.store.status()?;
            let json = serde_json::json!({
                "path": s.path,
                "protectedBy": describe(s.key_source),
                "fileBytes": s.file_bytes,
            });
            return ctx.print(&json, |_| {
                if let Some(p) = &s.path {
                    println!("store: {}", p.display());
                }
                println!("protected by: {}", describe(s.key_source));
                println!("size: {} bytes", s.file_bytes);
            });
        }
        StoreCommand::Passphrase => {
            let p = new_passphrase(true)?;
            let old = old_keychain(before)?;
            ctx.store.change_key(
                KeySetup::Passphrase {
                    passphrase: &p,
                    kdf: KdfParams::DEFAULT,
                },
                old.as_ref().map(|k| k as _),
            )?;
        }
        StoreCommand::Keychain => {
            let keychain = OsKeychain::new().context("no system keychain is available")?;
            ctx.store
                .change_key(KeySetup::Keychain(&keychain), Some(&keychain))?;
        }
    }
    ctx.done("store key changed")
}

/// The keychain holding the current key, so the change can remove it.
fn old_keychain(source: Option<KeySource>) -> Result<Option<OsKeychain>> {
    Ok(match source {
        Some(KeySource::Keychain) => Some(OsKeychain::new()?),
        _ => None,
    })
}
