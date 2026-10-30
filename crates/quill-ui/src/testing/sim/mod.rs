//! A simulated Quill server and client store behind the `Backend` trait,
//! for interaction tests. It keeps state like the real thing (logins,
//! personas, keys on this computer, rules and their versions, locked
//! stores) and a test can change it mid-flow, as an administrator or a
//! network outage would. It follows `quill-gui`'s `NativeBackend` closely;
//! the end-to-end tests check the real one against a real server.

use std::{cell::RefCell, collections::HashMap, future::Future, pin::Pin, rc::Rc, time::Duration};

use quill_proto::board::Maturity;
use uuid::Uuid;

mod board;

use crate::backend::{
    AccountNotice, AccountSettings, CreatePrompt, CreateReport, Prompt, PromptPage, PromptQuery,
    Tag, TagNamespace, TagPage, TagPageQuery, TagPolicy, TagQuery, UpdatePrompt,
};
use crate::backend::{
    AppSettings, Backend, BoardSettings, Call, Details, Keys, PASSPHRASE_MIN_CHARS, Persona,
    PersonaState, Problem, Protection, ServerEntry, ServerInfo, ServerRules, Standing, Step, Theme,
};

/// When accounts must accept a server's rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Acceptance {
    #[default]
    Never,
    OnJoin,
    OnChange,
    Both,
}

impl Acceptance {
    fn on_join(self) -> bool {
        matches!(self, Self::OnJoin | Self::Both)
    }
    fn on_change(self) -> bool {
        matches!(self, Self::OnChange | Self::Both)
    }
}

#[derive(Debug, Clone)]
struct Account {
    password: String,
    accepted: u32,
    /// The board's maturity cap for this login.
    maturity: Maturity,
}

/// One simulated server.
#[derive(Debug, Clone)]
pub struct SimServer {
    pub domain: String,
    pub reachable: bool,
    pub registration_open: bool,
    rules: Option<(String, u32)>,
    pub acceptance: Acceptance,
    /// Fetching the rules fails (a flaky connection, say).
    pub rules_fail: bool,
    accounts: HashMap<String, Account>,
    /// Personas by login, with whether their identity key is published.
    personas: Vec<(String, Persona, bool)>,
    board: board::Board,
}

impl SimServer {
    fn new(domain: &str) -> Self {
        Self {
            domain: domain.into(),
            reachable: true,
            registration_open: true,
            rules: None,
            acceptance: Acceptance::Never,
            rules_fail: false,
            accounts: HashMap::new(),
            personas: Vec::new(),
            board: board::Board::default(),
        }
    }
}

/// This computer's copy for one server.
#[derive(Debug, Clone)]
struct Store {
    address: String,
    login: Option<String>,
    signed_in: bool,
    passphrase: Option<String>,
    unlocked: bool,
    /// Personas whose keys are on this computer.
    keys: Vec<Uuid>,
}

#[derive(Debug)]
struct State {
    servers: HashMap<String, SimServer>,
    entries: Vec<ServerEntry>,
    stores: HashMap<Uuid, Store>,
    current: Option<Uuid>,
    keychain: bool,
    /// Wakes waits of a minute or more (`time_passes`).
    clock: Rc<tokio::sync::Notify>,
    settings: AppSettings,
}

/// The simulated world: servers out there and this computer's app state.
/// Clone the `Rc` to keep a handle for changing it during a test.
pub struct Sim(RefCell<State>);

/// A server address as the app stores it.
fn address(domain: &str) -> String {
    format!("https://{domain}")
}

impl Default for Sim {
    fn default() -> Self {
        Self::new()
    }
}

impl Sim {
    pub fn new() -> Self {
        Self(RefCell::new(State {
            servers: HashMap::new(),
            entries: Vec::new(),
            stores: HashMap::new(),
            current: None,
            keychain: true,
            clock: Rc::default(),
            settings: AppSettings {
                theme: Theme::System,
                text_size: 100,
                advanced: false,
                stores_folder: "/home/writer/.local/share/quill-gui/stores".into(),
            },
        }))
    }

    pub fn rc(self) -> Rc<Self> {
        Rc::new(self)
    }

    /// Adds a server out there, reachable at `domain`.
    pub fn with_server(self, domain: &str) -> Self {
        self.0
            .borrow_mut()
            .servers
            .insert(address(domain), SimServer::new(domain));
        self
    }

    /// No OS keychain: stores need a passphrase.
    pub fn without_keychain(self) -> Self {
        self.0.borrow_mut().keychain = false;
        self
    }

    /// Every wait of a minute or more (periodic checks, upkeep) ends now,
    /// as if that long had passed. Such waits otherwise last until called.
    pub fn time_passes(&self) {
        self.0.borrow().clock.notify_waiters();
    }

    /// Changes a server (an administrator's settings, an outage).
    pub fn server<T>(&self, domain: &str, change: impl FnOnce(&mut SimServer) -> T) -> T {
        let mut s = self.0.borrow_mut();
        let server = s
            .servers
            .get_mut(&address(domain))
            .unwrap_or_else(|| panic!("no server {domain}"));
        change(server)
    }

    /// Sets or changes a server's rules; each change is a new version.
    pub fn set_rules(&self, domain: &str, markdown: &str, acceptance: Acceptance) {
        self.server(domain, |s| {
            let version = s.rules.as_ref().map_or(1, |(_, v)| v + 1);
            s.rules = Some((markdown.into(), version));
            s.acceptance = acceptance;
        });
    }

    /// A login that already exists on a server, with personas whose keys
    /// are on another device.
    pub fn existing_login(&self, domain: &str, login: &str, password: &str, handles: &[&str]) {
        self.server(domain, |s| {
            s.accounts.insert(
                login.into(),
                Account {
                    password: password.into(),
                    accepted: 0,
                    maturity: Maturity::DEFAULT_MAX,
                },
            );
            for h in handles {
                s.personas.push((login.into(), persona(h), true));
            }
        });
    }

    /// Takes a handle on a server, as another writer would.
    pub fn take_handle(&self, domain: &str, handle: &str) {
        self.existing_login(
            domain,
            &format!("owner_of_{handle}"),
            "a long password",
            &[handle],
        );
    }

    /// The login names a server has.
    pub fn logins(&self, domain: &str) -> Vec<String> {
        let s = self.0.borrow();
        let mut v: Vec<String> = s.servers[&address(domain)]
            .accounts
            .keys()
            .cloned()
            .collect();
        v.sort();
        v
    }

    /// Which rules version `login` has accepted on a server (0: none).
    pub fn accepted(&self, domain: &str, login: &str) -> u32 {
        self.0.borrow().servers[&address(domain)].accounts[login].accepted
    }

    fn open_store(&self) -> Result<(Uuid, Store, SimServer), Problem> {
        let s = self.0.borrow();
        let id = s.current.ok_or_else(|| Problem::Failed {
            detail: "no server is open".into(),
        })?;
        let store = s.stores[&id].clone();
        let server = s.servers[&store.address].clone();
        Ok((id, store, server))
    }

    fn reach(&self) -> Result<(Uuid, Store, SimServer), Problem> {
        let (id, store, server) = self.open_store()?;
        if !server.reachable {
            return Err(Problem::Unreachable(server.domain));
        }
        Ok((id, store, server))
    }

    fn standing_now(&self) -> Result<Standing, Problem> {
        let (_, store, server) = self.open_store()?;
        if store.passphrase.is_some() && !store.unlocked {
            return Ok(Standing::Locked);
        }
        if !store.signed_in {
            return Ok(Standing::SignedOut);
        }
        if !server.reachable {
            return Err(Problem::Unreachable(server.domain));
        }
        let login = store.login.clone().unwrap_or_default();
        let personas = server
            .personas
            .iter()
            .filter(|(owner, _, _)| *owner == login)
            .map(|(_, p, published)| PersonaState {
                persona: p.clone(),
                keys: if store.keys.contains(&p.id) {
                    Keys::Ready
                } else if *published {
                    Keys::Elsewhere
                } else {
                    Keys::ToSetUp
                },
            })
            .collect();
        Ok(Standing::SignedIn { login, personas })
    }

    fn with_server_mut<T>(&self, f: impl FnOnce(&mut SimServer, &mut Store) -> T) -> T {
        let mut s = self.0.borrow_mut();
        let id = s.current.expect("a server is open");
        let st = &mut *s;
        let store = st.stores.get_mut(&id).expect("its store");
        let server = st.servers.get_mut(&store.address).expect("its server");
        f(server, store)
    }
}

/// A persona as the server returns it.
pub fn persona(handle: &str) -> Persona {
    Persona {
        id: Uuid::new_v4(),
        handle: handle.into(),
        actor_uri: format!("https://sim.example/actors/{handle}"),
        display_name: None,
        summary: None,
        created_at: time::OffsetDateTime::UNIX_EPOCH,
    }
}

fn now<'a, T: 'a>(v: Result<T, Problem>) -> Call<'a, T> {
    Box::pin(async move {
        // A moment's latency, as a network would have, so views see their
        // in-between states.
        tokio::time::sleep(Duration::from_millis(5)).await;
        v
    })
}

fn refused(text: &str) -> Problem {
    Problem::Refused(text.into())
}

/// The address as typed, as the app normalizes it (scheme optional).
fn normalize(typed: &str) -> String {
    let t = typed.trim().trim_end_matches('/');
    let bare = t
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    address(bare)
}

impl Backend for Sim {
    fn servers(&self) -> Vec<ServerEntry> {
        self.0.borrow().entries.clone()
    }

    fn current(&self) -> Option<Uuid> {
        self.0.borrow().current
    }

    fn check_server<'a>(&'a self, typed: &'a str) -> Call<'a, ServerInfo> {
        let result = (|| {
            if typed.trim().is_empty() {
                return Err(refused("Type the server's address."));
            }
            let addr = normalize(typed);
            let s = self.0.borrow();
            let server = s
                .servers
                .get(&addr)
                .filter(|s| s.reachable)
                .ok_or_else(|| Problem::Unreachable(addr.clone()))?;
            Ok(ServerInfo {
                address: addr.clone(),
                domain: server.domain.clone(),
                registration_open: server.registration_open,
                has_rules: server.rules.is_some(),
                rules_to_accept: server.rules.is_some() && server.acceptance != Acceptance::Never,
                software: "quill".into(),
                version: "0.0.0-sim".into(),
            })
        })();
        now(result)
    }

    fn add_server<'a>(
        &'a self,
        typed: &'a str,
        passphrase: Option<&'a str>,
    ) -> Call<'a, ServerEntry> {
        let result = (|| {
            let info = {
                let addr = normalize(typed);
                let s = self.0.borrow();
                let server = s
                    .servers
                    .get(&addr)
                    .filter(|s| s.reachable)
                    .ok_or_else(|| Problem::Unreachable(addr.clone()))?;
                (addr, server.domain.clone())
            };
            let mut s = self.0.borrow_mut();
            if s.entries.iter().any(|e| e.address == info.0) {
                return Err(refused(&format!(
                    "{} is already in your list of servers.",
                    info.1
                )));
            }
            match passphrase {
                Some(p) if p.chars().count() < PASSPHRASE_MIN_CHARS => {
                    return Err(refused("Use at least 12 characters."));
                }
                None if !s.keychain => return Err(Problem::NeedsPassphrase),
                _ => {}
            }
            let entry = ServerEntry {
                id: Uuid::new_v4(),
                address: info.0.clone(),
                name: info.1,
                login: None,
            };
            s.entries.push(entry.clone());
            s.stores.insert(
                entry.id,
                Store {
                    address: info.0,
                    login: None,
                    signed_in: false,
                    passphrase: passphrase.map(str::to_owned),
                    unlocked: true,
                    keys: Vec::new(),
                },
            );
            s.current = Some(entry.id);
            Ok(entry)
        })();
        now(result)
    }

    fn open_server(&self, id: Uuid) -> Call<'_, Standing> {
        self.0.borrow_mut().current = Some(id);
        now(self.standing_now())
    }

    fn unlock<'a>(&'a self, passphrase: &'a str) -> Call<'a, Standing> {
        let ok = self.with_server_mut(|_, store| {
            let ok = store.passphrase.as_deref() == Some(passphrase);
            if ok {
                store.unlocked = true;
            }
            ok
        });
        if !ok {
            return now(Err(refused("That isn't the passphrase.")));
        }
        now(self.standing_now())
    }

    fn remove_server(&self, id: Uuid) -> Call<'_, ()> {
        let mut s = self.0.borrow_mut();
        s.entries.retain(|e| e.id != id);
        s.stores.remove(&id);
        if s.current == Some(id) {
            s.current = None;
        }
        now(Ok(()))
    }

    fn rename_server(&self, id: Uuid, name: &str) -> Result<(), Problem> {
        let name = name.trim();
        if name.is_empty() {
            return Err(refused("Use 1 to 60 characters."));
        }
        if let Some(e) = self.0.borrow_mut().entries.iter_mut().find(|e| e.id == id) {
            e.name = name.into();
        }
        Ok(())
    }

    fn keychain_available(&self) -> bool {
        self.0.borrow().keychain
    }

    fn protection(&self) -> Option<Protection> {
        let (_, store, _) = self.open_store().ok()?;
        Some(if store.passphrase.is_some() {
            Protection::Passphrase
        } else {
            Protection::Keychain
        })
    }

    fn set_protection<'a>(&'a self, passphrase: Option<&'a str>) -> Call<'a, ()> {
        if passphrase.is_none() && !self.0.borrow().keychain {
            return now(Err(Problem::NeedsPassphrase));
        }
        if passphrase.is_some_and(|p| p.chars().count() < PASSPHRASE_MIN_CHARS) {
            return now(Err(refused("Use at least 12 characters.")));
        }
        self.with_server_mut(|_, store| store.passphrase = passphrase.map(str::to_owned));
        now(Ok(()))
    }

    fn standing(&self) -> Call<'_, Standing> {
        now(self.standing_now())
    }

    fn register<'a>(
        &'a self,
        login: &'a str,
        password: &'a str,
        accept_rules: Option<u32>,
    ) -> Call<'a, Standing> {
        if let Err(p) = self.reach() {
            return now(Err(p));
        }
        let login = login.trim().to_ascii_lowercase();
        let result = self.with_server_mut(|server, store| {
            if !server.registration_open {
                return Err(refused("This server isn't taking new logins."));
            }
            if server.accounts.contains_key(&login) {
                return Err(Problem::Taken);
            }
            let mut accepted = 0;
            if let Some((_, version)) = &server.rules {
                match accept_rules {
                    Some(v) if v == *version => accepted = v,
                    Some(_) if server.acceptance.on_join() => {
                        return Err(Problem::RulesNotAccepted(
                            "The server rules changed while you were signing up. Read them again."
                                .into(),
                        ));
                    }
                    None if server.acceptance.on_join() => {
                        return Err(Problem::RulesNotAccepted(
                            "Accept the server rules to create an account.".into(),
                        ));
                    }
                    _ => {}
                }
            }
            server.accounts.insert(
                login.clone(),
                Account {
                    password: password.into(),
                    accepted,
                    maturity: Maturity::DEFAULT_MAX,
                },
            );
            store.login = Some(login.clone());
            store.signed_in = true;
            Ok(())
        });
        if let Err(p) = result {
            return now(Err(p));
        }
        self.set_entry_login(Some(login));
        now(self.standing_now())
    }

    fn sign_in<'a>(&'a self, login: &'a str, password: &'a str) -> Call<'a, Standing> {
        if let Err(p) = self.reach() {
            return now(Err(p));
        }
        let login = login.trim().to_ascii_lowercase();
        let ok = self.with_server_mut(|server, store| {
            let ok = server
                .accounts
                .get(&login)
                .is_some_and(|a| a.password == password);
            if ok {
                store.login = Some(login.clone());
                store.signed_in = true;
            }
            ok
        });
        if !ok {
            return now(Err(refused("That login name and password don't match.")));
        }
        self.set_entry_login(Some(login));
        now(self.standing_now())
    }

    fn sign_out(&self) -> Call<'_, ()> {
        self.with_server_mut(|_, store| store.signed_in = false);
        now(Ok(()))
    }

    fn rules(&self) -> Call<'_, Option<ServerRules>> {
        if self.open_store().is_ok_and(|(_, _, s)| s.rules_fail) {
            return now(Err(Problem::Failed {
                detail: "simulated failure".into(),
            }));
        }
        let result = self.reach().map(|(_, store, server)| {
            server
                .rules
                .as_ref()
                .map(|(markdown, version)| ServerRules {
                    markdown: markdown.clone(),
                    version: *version,
                    must_accept: store.signed_in
                        && server.acceptance.on_change()
                        && store
                            .login
                            .as_ref()
                            .and_then(|l| server.accounts.get(l))
                            .is_some_and(|a| a.accepted < *version),
                })
        });
        now(result)
    }

    fn accept_rules(&self, version: u32) -> Call<'_, ()> {
        if let Err(p) = self.reach() {
            return now(Err(p));
        }
        let result = self.with_server_mut(|server, store| {
            let current = server.rules.as_ref().map_or(0, |(_, v)| *v);
            if version != current {
                return Err(Problem::RulesNotAccepted(
                    "The rules changed again while you were reading. Read the new version.".into(),
                ));
            }
            let login = store.login.clone().unwrap_or_default();
            if let Some(a) = server.accounts.get_mut(&login) {
                a.accepted = version;
            }
            Ok(())
        });
        now(result)
    }

    fn create_persona<'a>(&'a self, handle: &'a str, display_name: &'a str) -> Call<'a, Persona> {
        if let Err(p) = self.reach() {
            return now(Err(p));
        }
        let handle = handle.trim().to_ascii_lowercase();
        let result = self.with_server_mut(|server, store| {
            if server.personas.iter().any(|(_, p, _)| p.handle == handle) {
                return Err(Problem::Taken);
            }
            let mut p = persona(&handle);
            p.display_name = Some(display_name.trim().to_owned()).filter(|d| !d.is_empty());
            server
                .personas
                .push((store.login.clone().unwrap_or_default(), p.clone(), false));
            Ok(p)
        });
        now(result)
    }

    fn update_persona<'a>(
        &'a self,
        id: Uuid,
        display_name: &'a str,
        summary: &'a str,
    ) -> Call<'a, Persona> {
        let result = self.with_server_mut(|server, _| {
            let (_, p, _) = server
                .personas
                .iter_mut()
                .find(|(_, p, _)| p.id == id)
                .ok_or_else(|| refused("That persona is gone."))?;
            p.display_name = Some(display_name.trim().to_owned()).filter(|d| !d.is_empty());
            p.summary = Some(summary.trim().to_owned()).filter(|d| !d.is_empty());
            Ok(p.clone())
        });
        now(result)
    }

    fn delete_persona(&self, id: Uuid) -> Call<'_, ()> {
        self.with_server_mut(|server, _| server.personas.retain(|(_, p, _)| p.id != id));
        now(Ok(()))
    }

    fn prepare<'a>(&'a self, persona: Uuid, progress: &'a dyn Fn(Step)) -> Call<'a, ()> {
        Box::pin(async move {
            for step in Step::ALL {
                progress(step);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.reach()?;
            self.with_server_mut(|server, store| {
                let (_, _, published) = server
                    .personas
                    .iter_mut()
                    .find(|(_, p, _)| p.id == persona)
                    .ok_or_else(|| refused("That persona is gone."))?;
                if *published && !store.keys.contains(&persona) {
                    return Err(refused(
                        "This persona's keys are on another of your devices. Link this computer to it, or restore a backup.",
                    ));
                }
                *published = true;
                store.keys.push(persona);
                Ok(())
            })
        })
    }

    fn upkeep(&self) -> Call<'_, Option<String>> {
        now(self.reach().map(|_| None))
    }

    fn next_notice(&self) -> Call<'_, ()> {
        Box::pin(std::future::pending())
    }

    fn board_settings(&self) -> Call<'_, BoardSettings> {
        now(self.signed_in().map(|(login, server)| BoardSettings {
            maturity_max: server.account_of(&login).maturity_max,
        }))
    }

    fn set_board_settings(&self, settings: BoardSettings) -> Call<'_, BoardSettings> {
        self.with_server_mut(|server, store| {
            if let Some(a) = store
                .login
                .as_ref()
                .and_then(|l| server.accounts.get_mut(l))
            {
                a.maturity = settings.maturity_max;
            }
        });
        now(Ok(settings))
    }

    fn account(&self) -> Call<'_, AccountSettings> {
        now(self
            .signed_in()
            .map(|(login, server)| server.account_of(&login)))
    }

    fn prompts(&self, query: PromptQuery) -> Call<'_, PromptPage> {
        now(self.reach().and_then(|(_, store, server)| {
            server.search(
                store.signed_in.then_some(store.login.as_deref()).flatten(),
                &query,
            )
        }))
    }

    fn prompt(&self, id: Uuid) -> Call<'_, Prompt> {
        now(self.reach().and_then(|(_, store, server)| {
            server.one(
                store.signed_in.then_some(store.login.as_deref()).flatten(),
                id,
            )
        }))
    }

    fn create_prompt(&self, body: CreatePrompt) -> Call<'_, Prompt> {
        now(self.writing(|server, login| server.post(login, &body)))
    }

    fn update_prompt(&self, id: Uuid, body: UpdatePrompt) -> Call<'_, Prompt> {
        now(self.writing(|server, login| server.edit(login, id, &body)))
    }

    fn delete_prompt(&self, id: Uuid) -> Call<'_, ()> {
        now(self.writing(|server, login| server.delete(login, id)))
    }

    fn prompt_link(&self, id: Uuid) -> Option<String> {
        let (_, store, _) = self.open_store().ok()?;
        Some(format!("{}/p/{id}", store.address))
    }

    fn tag_types(&self) -> Call<'_, Vec<TagNamespace>> {
        now(self.reach().map(|(_, _, server)| server.tag_types()))
    }

    fn tag_policy(&self) -> Call<'_, TagPolicy> {
        now(self.reach().map(|(_, _, server)| server.board.policy))
    }

    fn tag_page<'a>(&'a self, namespace: &'a str, query: TagPageQuery) -> Call<'a, TagPage> {
        now(self
            .reach()
            .and_then(|(_, _, server)| server.tag_page(namespace, &query)))
    }

    fn find_tags(&self, query: TagQuery) -> Call<'_, Vec<Tag>> {
        now(self.reach().map(|(_, _, server)| server.find(&query)))
    }

    fn frequent_tags(&self, persona: Option<Uuid>) -> Call<'_, Vec<Tag>> {
        now(self
            .signed_in()
            .map(|(login, server)| server.frequent(&login, persona)))
    }

    fn report(&self, body: CreateReport) -> Call<'_, ()> {
        if let Err(p) = self.reach() {
            return now(Err(p));
        }
        now(self.with_server_mut(|server, store| {
            let login = store.signed_in.then(|| store.login.clone()).flatten();
            server.file_report(login.as_deref(), body)
        }))
    }

    fn notices(&self, before: Option<i64>) -> Call<'_, Vec<AccountNotice>> {
        now(self
            .signed_in()
            .map(|(login, server)| server.notices_of(&login, before)))
    }

    fn mark_notices_read(&self) -> Call<'_, ()> {
        now(self.writing(|server, login| {
            server.read_notices(login);
            Ok(())
        }))
    }

    fn app_settings(&self) -> AppSettings {
        self.0.borrow().settings.clone()
    }

    fn set_app_settings(&self, settings: AppSettings) -> Result<(), Problem> {
        self.0.borrow_mut().settings = settings;
        Ok(())
    }

    fn move_stores<'a>(&'a self, folder: &'a str) -> Call<'a, ()> {
        self.0.borrow_mut().settings.stores_folder = folder.trim().into();
        now(Ok(()))
    }

    fn details(&self) -> Call<'_, Details> {
        let rows = match self.open_store() {
            Ok((id, store, _)) => vec![
                ("Server id (this app's)".to_owned(), id.to_string()),
                ("Store".to_owned(), format!("/stores/{id}.quill")),
                (
                    "Store key".to_owned(),
                    if store.passphrase.is_some() {
                        "passphrase"
                    } else {
                        "system keychain"
                    }
                    .into(),
                ),
            ],
            Err(_) => Vec::new(),
        };
        now(Ok(Details { rows }))
    }

    fn sleep(&self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + '_>> {
        if ms >= 60_000 {
            let clock = self.0.borrow().clock.clone();
            return Box::pin(async move { clock.notified().await });
        }
        Box::pin(tokio::time::sleep(Duration::from_millis(ms)))
    }
}

impl Sim {
    /// The open server and its signed-in login, or why there's none.
    fn signed_in(&self) -> Result<(String, SimServer), Problem> {
        let (_, store, server) = self.reach()?;
        match store.login {
            Some(login) if store.signed_in => Ok((login, server)),
            _ => Err(Problem::SignedOut),
        }
    }

    /// Runs a change on the open server as its signed-in login.
    fn writing<T>(
        &self,
        f: impl FnOnce(&mut SimServer, &str) -> Result<T, Problem>,
    ) -> Result<T, Problem> {
        self.signed_in()?;
        self.with_server_mut(|server, store| {
            let login = store.login.clone().unwrap_or_default();
            f(server, &login)
        })
    }

    /// The app closed: stores with a passphrase are locked again.
    pub fn relock(&self) {
        for store in self.0.borrow_mut().stores.values_mut() {
            if store.passphrase.is_some() {
                store.unlocked = false;
            }
        }
    }

    fn set_entry_login(&self, login: Option<String>) {
        let mut s = self.0.borrow_mut();
        if let Some(id) = s.current
            && let Some(e) = s.entries.iter_mut().find(|e| e.id == id)
        {
            e.login = login;
        }
    }
}
