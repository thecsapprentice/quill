//! What the views need from a client: implemented over `quill-core` by
//! `quill-gui` (and later the web client), and by a fake in tests.
//!
//! Every call is async and returns a [`Problem`] the views can show as is:
//! the backend turns `quill-core`'s errors into plain words, so no view
//! ever shows a raw error outside advanced mode.

use std::{future::Future, pin::Pin};

use quill_proto::board::Maturity;
pub use quill_proto::{
    api::Persona,
    board::{
        AccountSettings, CreatePrompt, CreateReport, Prompt, PromptPage, PromptQuery, Tag,
        TagNamespace, TagPage, TagPageQuery, TagPolicy, TagQuery, UpdatePrompt,
    },
    notices::AccountNotice,
};
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
    /// What was asked for is gone: deleted, or no longer shown to you.
    #[error("That's gone: it was deleted, or isn't shown to you any more.")]
    Gone,
    /// The server rules must be accepted first (read them, then accept):
    /// on joining, or after they change.
    #[error("{0}")]
    RulesNotAccepted(String),
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
    /// Whether the server has rules.
    pub has_rules: bool,
    /// Whether a new login must accept them: the server asks on joining,
    /// or will ask before its first post. Sign-up asks either way.
    pub rules_to_accept: bool,
    /// For advanced mode.
    pub software: String,
    pub version: String,
}

/// The open server's rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRules {
    /// Markdown, which the client renders itself (never the server's HTML).
    pub markdown: String,
    /// Accepting names the version read.
    pub version: u32,
    /// Signed in: whether this login must accept this version to keep
    /// posting, editing, commenting and reporting.
    pub must_accept: bool,
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

/// A link this computer opened, waiting for a device already in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkStart {
    /// The whole `quill-link:` code, for the QR code and for copying.
    pub code: String,
    /// The six words to type on the device in use.
    pub words: Vec<String>,
    /// How long the link lasts.
    pub expires_in_secs: u64,
}

/// What approving a new computer's link came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Approval {
    /// Linked: both devices should now show these words.
    Approved { confirm: Vec<String> },
    /// No new computer is waiting (or its link expired).
    NoneWaiting,
    /// More than one is waiting: someone else may be linking, so none is
    /// approved by words alone.
    Several(usize),
    /// The words don't match the computer that's waiting.
    WordsDiffer,
}

/// One of the login's devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: Uuid,
    pub this_device: bool,
    pub name: Option<String>,
    /// When it joined the login's devices, in seconds since 1970.
    pub since: Option<u64>,
    /// The personas it can write as, by handle.
    pub personas: Vec<String>,
}

/// Why a device is removed, recorded with its persona keys' revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RemoveReason {
    Lost,
    Compromised,
    Retired,
    #[default]
    Unspecified,
}

impl RemoveReason {
    pub const ALL: [RemoveReason; 4] = [
        RemoveReason::Retired,
        RemoveReason::Lost,
        RemoveReason::Compromised,
        RemoveReason::Unspecified,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RemoveReason::Retired => "I don't use it any more",
            RemoveReason::Lost => "It was lost or stolen",
            RemoveReason::Compromised => "Someone else may have used it",
            RemoveReason::Unspecified => "I'd rather not say",
        }
    }
}

/// A persona's keys as advanced mode shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaKeys {
    pub handle: String,
    /// Label and value pairs, in order.
    pub rows: Vec<(String, String)>,
}

/// The longest device name, in characters (as `quill-core` allows).
pub const DEVICE_NAME_MAX: usize = 40;

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
    /// Makes a login. `accept_rules` is the rules version shown at sign-up,
    /// when the writer accepted it.
    fn register<'a>(
        &'a self,
        login: &'a str,
        password: &'a str,
        accept_rules: Option<u32>,
    ) -> Call<'a, Standing>;
    fn sign_in<'a>(&'a self, login: &'a str, password: &'a str) -> Call<'a, Standing>;
    fn sign_out(&self) -> Call<'_, ()>;
    /// The open server's rules, or `None` if it has none.
    fn rules(&self) -> Call<'_, Option<ServerRules>>;
    fn accept_rules(&self, version: u32) -> Call<'_, ()>;

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
    /// As `prepare`, but makes a new identity key in place of one made on
    /// another device. That device stops working for the persona, and
    /// other writers are told the key changed.
    fn start_over<'a>(&'a self, persona: Uuid, progress: &'a dyn Fn(Step)) -> Call<'a, ()>;

    // ---- Devices ---------------------------------------------------------------------------

    /// What this computer is called on the login's other devices.
    fn device_name(&self) -> Option<String>;
    /// Renames this computer, telling the other devices if it's linked.
    fn rename_device<'a>(&'a self, name: &'a str) -> Call<'a, ()>;
    /// The login's devices; empty until a second one is linked.
    fn devices(&self) -> Call<'_, Vec<Device>>;
    /// Removes another of the login's devices; its persona keys are revoked.
    fn remove_device(&self, id: Uuid, reason: RemoveReason) -> Call<'_, ()>;
    /// On a new computer: names it and opens a link for a device in use.
    fn start_link<'a>(&'a self, name: &'a str) -> Call<'a, LinkStart>;
    /// Whether the link was answered: the words to compare, `None` while
    /// waiting (it returns within a few seconds), `Gone` once it expired.
    fn link_answer(&self) -> Call<'_, Option<Vec<String>>>;
    /// The words matched: takes the keys and gets the personas ready.
    fn finish_link(&self) -> Call<'_, Standing>;
    /// The words didn't match, or the person gave up: takes nothing.
    fn abandon_link(&self) -> Result<(), Problem>;
    /// On a device in use: approves the waiting link whose words (or whole
    /// code) are `typed`.
    fn approve_link<'a>(&'a self, typed: &'a str) -> Call<'a, Approval>;
    /// How many new computers are waiting to link to this login.
    fn waiting_links(&self) -> Call<'_, usize>;
    /// Each persona's keys, for advanced mode.
    fn key_details(&self) -> Call<'_, Vec<PersonaKeys>>;

    // ---- The board ------------------------------------------------------------------------

    /// The login's board settings and unread notice count.
    fn account(&self) -> Call<'_, AccountSettings>;
    /// One page of the board, newest first.
    fn prompts(&self, query: PromptQuery) -> Call<'_, PromptPage>;
    fn prompt(&self, id: Uuid) -> Call<'_, Prompt>;
    fn create_prompt(&self, body: CreatePrompt) -> Call<'_, Prompt>;
    fn update_prompt(&self, id: Uuid, body: UpdatePrompt) -> Call<'_, Prompt>;
    fn delete_prompt(&self, id: Uuid) -> Call<'_, ()>;
    /// The board's address for a prompt, to share.
    fn prompt_link(&self, id: Uuid) -> Option<String>;
    fn tag_types(&self) -> Call<'_, Vec<TagNamespace>>;
    /// Whether writers may add new tags, and whether those need approval.
    fn tag_policy(&self) -> Call<'_, TagPolicy>;
    /// One page of a tag type's tags with their prompt counts.
    fn tag_page<'a>(&'a self, namespace: &'a str, query: TagPageQuery) -> Call<'a, TagPage>;
    /// Tags whose names start with `query.prefix` (the tag picker).
    fn find_tags(&self, query: TagQuery) -> Call<'_, Vec<Tag>>;
    /// The tags the login uses most, for `persona`.
    fn frequent_tags(&self, persona: Option<Uuid>) -> Call<'_, Vec<Tag>>;
    fn report(&self, body: CreateReport) -> Call<'_, ()>;
    /// The login's notices, newest first, before notice `before`.
    fn notices(&self, before: Option<i64>) -> Call<'_, Vec<AccountNotice>>;
    fn mark_notices_read(&self) -> Call<'_, ()>;

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
