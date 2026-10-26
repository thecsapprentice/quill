//! What the views need from a client: implemented over `quill-core` by
//! `quill-gui` (and later the web client), and by a fake in tests.
//!
//! Every call is async and returns a [`Problem`] the views can show as is:
//! the backend turns `quill-core`'s errors into plain words, so no view
//! ever shows a raw error outside advanced mode.

use std::{future::Future, pin::Pin};

pub use quill_proto::api::Persona;
use quill_proto::board::Maturity;
use uuid::Uuid;

/// The shortest store passphrase, as in the CLI.
pub const PASSPHRASE_MIN_CHARS: usize = 12;

/// A backend call in flight. Not `Send`: the web client's futures aren't.
pub type Call<'a, T> = Pin<Box<dyn Future<Output = Result<T, Problem>> + 'a>>;

/// Something went wrong, said for the writer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    /// The server couldn't be reached: offline, or the address is wrong.
    #[error("Can't reach {0} right now.")]
    Unreachable(String),
    /// The login's token is gone or expired: sign in again.
    #[error("You're signed out of this server.")]
    SignedOut,
    /// This computer has no keychain to keep a store's key in: the writer
    /// must choose a passphrase (adding a server) or type it (opening one).
    #[error("This computer has no keychain for Quill's key; use a passphrase.")]
    NeedsPassphrase,
    /// The name asked for (a login or a handle) isn't available.
    #[error("That name isn't available.")]
    Taken,
    /// The server refused, with its reason (already in plain words).
    #[error("{0}")]
    Refused(String),
    /// Anything else. `detail` is shown only in advanced mode.
    #[error("Something went wrong.")]
    Failed { detail: String },
}

impl Problem {
    /// The underlying error, for advanced mode.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Problem::Failed { detail } => Some(detail),
            _ => None,
        }
    }
}

/// A server this app knows, from its settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerEntry {
    /// The app's own id for the entry (not the server's).
    pub id: Uuid,
    /// As typed, normalized: `https://writers.example`.
    pub address: String,
    /// The server's domain, or a name the writer gave it.
    pub name: String,
    /// The login used there, once signed in.
    pub login: Option<String>,
}

/// What a server says about itself, before signing in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub address: String,
    pub domain: String,
    pub registration_open: bool,
    /// For advanced mode.
    pub software: String,
    pub version: String,
}

/// Where the open server's login stands, which decides what the app shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// The store is protected by a passphrase not yet typed this session.
    Locked,
    /// No token, or the server refused it.
    SignedOut,
    /// Signed in. `personas` with keys on this computer are ready to use.
    SignedIn {
        login: String,
        personas: Vec<PersonaState>,
    },
}

/// One persona and whether this computer can write as it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaState {
    pub persona: Persona,
    pub keys: Keys,
}

/// The state of a persona's keys on this computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    /// Ready: an identity key and a certified device here.
    Ready,
    /// Nothing published yet, or only part of it: getting ready can finish.
    ToSetUp,
    /// The persona's identity key lives on another device: link this
    /// computer to it (3d) or restore a backup (3h).
    Elsewhere,
}

/// A step of getting a persona ready, for the progress screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    IdentityKey,
    ThisComputer,
    KeyPackages,
}

impl Step {
    pub const ALL: [Step; 3] = [Step::IdentityKey, Step::ThisComputer, Step::KeyPackages];

    pub fn label(self) -> &'static str {
        match self {
            Step::IdentityKey => "Making your persona's identity key",
            Step::ThisComputer => "Certifying this computer",
            Step::KeyPackages => "Getting ready for writing sessions",
        }
    }
}

/// The app's own settings (outside any store).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    pub theme: Theme,
    /// Text size in percent of the default.
    pub text_size: u16,
    pub advanced: bool,
    /// The folder holding the stores, for advanced mode.
    pub stores_folder: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// How the open server's store is locked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protection {
    /// By a key in the OS keychain: nothing to type.
    Keychain,
    /// By a passphrase typed when the app opens it.
    Passphrase,
}

/// Board settings kept on the server for the login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardSettings {
    pub maturity_max: Maturity,
}

/// What advanced mode shows about the open server and this computer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Details {
    /// Label and value pairs, in order.
    pub rows: Vec<(String, String)>,
}

/// The client's side of the app.
pub trait Backend {
    // ---- Servers -------------------------------------------------------------------

    /// The servers this app knows, in the order added.
    fn servers(&self) -> Vec<ServerEntry>;
    /// The server open now, if any.
    fn current(&self) -> Option<Uuid>;
    /// Asks a server about itself (for the address field, as you type).
    fn check_server<'a>(&'a self, address: &'a str) -> Call<'a, ServerInfo>;
    /// Adds a server (making its store) and opens it. The store's key goes
    /// in the keychain, or under `passphrase` if one is given (after a
    /// [`Problem::NeedsPassphrase`]).
    fn add_server<'a>(
        &'a self,
        address: &'a str,
        passphrase: Option<&'a str>,
    ) -> Call<'a, ServerEntry>;
    /// Opens a known server: unlocks its store and returns where its login
    /// stands.
    fn open_server(&self, id: Uuid) -> Call<'_, Standing>;
    /// Unlocks the open server's store with its passphrase.
    fn unlock<'a>(&'a self, passphrase: &'a str) -> Call<'a, Standing>;
    /// Forgets a server and deletes its store (and its keychain key).
    fn remove_server(&self, id: Uuid) -> Call<'_, ()>;
    /// Gives a server a name of the writer's choosing.
    fn rename_server(&self, id: Uuid, name: &str) -> Result<(), Problem>;

    // ---- The open server's store -------------------------------------------------------

    /// Whether this computer has a keychain to keep stores' keys in.
    fn keychain_available(&self) -> bool;
    /// How the open server's store is locked, once open.
    fn protection(&self) -> Option<Protection>;
    /// Locks the open server's store with `passphrase`, or with the
    /// keychain when `None`.
    fn set_protection<'a>(&'a self, passphrase: Option<&'a str>) -> Call<'a, ()>;

    // ---- The open server's login -----------------------------------------------------

    fn standing(&self) -> Call<'_, Standing>;
    fn register<'a>(&'a self, login: &'a str, password: &'a str) -> Call<'a, Standing>;
    fn sign_in<'a>(&'a self, login: &'a str, password: &'a str) -> Call<'a, Standing>;
    fn sign_out(&self) -> Call<'_, ()>;

    // ---- Personas ----------------------------------------------------------------------

    fn create_persona<'a>(&'a self, handle: &'a str, display_name: &'a str) -> Call<'a, Persona>;
    fn update_persona<'a>(
        &'a self,
        id: Uuid,
        display_name: &'a str,
        summary: &'a str,
    ) -> Call<'a, Persona>;
    fn delete_persona(&self, id: Uuid) -> Call<'_, ()>;
    /// Makes and publishes whatever `persona` lacks on this computer,
    /// calling `progress` as each step starts. Safe to call again after a
    /// failure: it carries on from where it stopped.
    fn prepare<'a>(&'a self, persona: Uuid, progress: &'a dyn Fn(Step)) -> Call<'a, ()>;

    // ---- Upkeep and settings --------------------------------------------------------------

    /// Background upkeep for the open server. Returns something to tell the
    /// writer, if anything needs them.
    fn upkeep(&self) -> Call<'_, Option<String>>;
    /// Waits for the server's next live notice for the login (a request,
    /// a session message, a device change). Upkeep runs after each.
    fn next_notice(&self) -> Call<'_, ()>;
    fn board_settings(&self) -> Call<'_, BoardSettings>;
    fn set_board_settings(&self, settings: BoardSettings) -> Call<'_, BoardSettings>;
    fn app_settings(&self) -> AppSettings;
    fn set_app_settings(&self, settings: AppSettings) -> Result<(), Problem>;
    /// Moves the stores to another folder (advanced).
    fn move_stores<'a>(&'a self, folder: &'a str) -> Call<'a, ()>;
    fn details(&self) -> Call<'_, Details>;
    /// Waits `ms` milliseconds, on whatever timer the platform has.
    fn sleep(&self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + '_>>;
}
