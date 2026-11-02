//! The native [`Backend`]: one encrypted store per server, the OS keychain,
//! and `quill-core` doing the work on a background runtime so the window
//! never waits on it.

use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use quill_core::{
    Client, CoreError, Store, StoreError, devices, keys,
    store::{KdfParams, KeySetup, KeySource, Unlock, vault::Vault},
};
use quill_proto::{
    api::{CreatePersona, Credentials, Persona, Registration, UpdatePersona},
    board::UpdateAccountSettings,
};
use quill_ui::backend::{
    AccountNotice, AccountSettings, CreatePrompt, CreateReport, Prompt, PromptPage, PromptQuery,
    Tag, TagNamespace, TagPage, TagPageQuery, TagPolicy, TagQuery, UpdatePrompt,
};
use quill_ui::backend::{
    AppSettings, Backend, BoardSettings, Call, Details, Keys, PASSPHRASE_MIN_CHARS, PersonaState,
    Problem, Protection, ServerEntry, ServerInfo, ServerRules, Standing, Step, Theme,
};
use tokio::runtime::Handle;
use uuid::Uuid;

use crate::settings::{self, Paths, Server, Settings};

/// How many upkeep results advanced mode keeps.
const UPKEEP_LOG: usize = 20;

/// The server open now.
#[derive(Clone)]
struct Open {
    id: Uuid,
    name: String,
    store: Arc<Store>,
    /// Signed in when the store holds a token.
    client: Client,
}

/// A server whose store waits for its passphrase.
#[derive(Clone)]
struct Locked {
    id: Uuid,
    path: PathBuf,
}

pub struct NativeBackend {
    paths: Paths,
    /// `None` where the OS has no usable keychain: stores then need a
    /// passphrase.
    vault: Option<Arc<dyn Vault>>,
    runtime: Handle,
    settings: Mutex<Settings>,
    open: Mutex<Option<Open>>,
    /// Every store unlocked this session, kept open so switching back to
    /// its server is instant (no passphrase or keychain prompt again).
    unlocked: Mutex<HashMap<Uuid, Open>>,
    locked: Mutex<Option<Locked>>,
    upkeep_log: Mutex<Vec<String>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // A panic elsewhere mustn't take the whole app down with it.
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn failed(detail: impl std::fmt::Display) -> Problem {
    Problem::Failed {
        detail: detail.to_string(),
    }
}

/// `quill-core`'s errors in the writer's words. `server` names the server
/// for "can't reach".
fn problem(server: &str, e: CoreError) -> Problem {
    match &e {
        CoreError::Http(h) if h.is_connect() || h.is_timeout() || h.is_request() => {
            Problem::Unreachable(server.to_owned())
        }
        CoreError::NotLoggedIn => Problem::SignedOut,
        CoreError::Api(p) if p.status == 401 => Problem::SignedOut,
        CoreError::Api(p) if p.status == 409 => Problem::Taken,
        CoreError::Api(p) if p.status == 404 || p.status == 410 => Problem::Gone,
        CoreError::Api(p) if p.kind.ends_with("/rules-not-accepted") => Problem::RulesNotAccepted(
            p.detail
                .clone()
                .unwrap_or_else(|| "Read and accept the server rules to continue.".into()),
        ),
        CoreError::Api(p) if (400..500).contains(&p.status) => {
            Problem::Refused(p.detail.clone().unwrap_or_else(|| p.title.clone()))
        }
        CoreError::Url(_) | CoreError::InsecureUrl => Problem::Refused(
            "That isn't a server address. It looks like writers.example or https://writers.example."
                .into(),
        ),
        _ => failed(e),
    }
}

/// Whether a typed address without a scheme names this computer.
fn is_local(address: &str) -> bool {
    let host = match address.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => address.split([':', '/']).next().unwrap_or(""),
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// A typed address as a URL, `https://` assumed when there's no scheme.
fn normalize(address: &str) -> Result<String, Problem> {
    let trimmed = address.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(Problem::Refused("Type the server's address.".into()));
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else if is_local(trimmed) {
        // A server on this computer (development) has no certificate.
        format!("http://{trimmed}")
    } else {
        format!("https://{trimmed}")
    };
    let url =
        quill_core::client::check_server_url(&with_scheme).map_err(|e| problem(trimmed, e))?;
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

/// Checks a new store passphrase against the CLI's rule.
fn check_passphrase(p: &str) -> Result<(), Problem> {
    if p.chars().count() < PASSPHRASE_MIN_CHARS {
        return Err(Problem::Refused(format!(
            "Use at least {PASSPHRASE_MIN_CHARS} characters."
        )));
    }
    Ok(())
}

/// A store still in use somewhere (a call in flight) can't be changed.
fn busy() -> Problem {
    Problem::Refused("Quill is still using this server's store. Try again in a moment.".into())
}

fn entry(s: &Server) -> ServerEntry {
    ServerEntry {
        id: s.id,
        address: s.address.clone(),
        name: s.name.clone(),
        login: s.login.clone(),
    }
}

impl NativeBackend {
    /// `vault` is the OS keychain in the app, a memory vault in tests.
    pub fn new(
        paths: Paths,
        vault: Option<Arc<dyn Vault>>,
        runtime: Handle,
    ) -> anyhow::Result<Self> {
        let settings = Settings::load(&paths.settings)?;
        Ok(Self {
            paths,
            vault,
            runtime,
            settings: Mutex::new(settings),
            open: Mutex::new(None),
            unlocked: Mutex::new(HashMap::new()),
            locked: Mutex::new(None),
            upkeep_log: Mutex::new(Vec::new()),
        })
    }

    fn stores_folder(&self) -> PathBuf {
        lock(&self.settings)
            .stores_folder
            .clone()
            .unwrap_or_else(|| self.paths.default_stores.clone())
    }

    fn store_path(&self, id: Uuid) -> PathBuf {
        self.stores_folder().join(format!("{id}.quill"))
    }

    fn save_settings(&self) -> Result<(), Problem> {
        let s = lock(&self.settings).clone();
        s.save(&self.paths.settings).map_err(failed)
    }

    fn opened(&self) -> Result<Open, Problem> {
        lock(&self.open)
            .clone()
            .ok_or_else(|| failed("no server is open"))
    }

    /// Runs a call on the open server's client, on the background runtime,
    /// with its errors in the writer's words.
    fn call<'a, T, F, Fut>(&'a self, work: F) -> Call<'a, T>
    where
        T: Send + 'static,
        F: FnOnce(Client) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, CoreError>> + Send + 'static,
    {
        let open = self.opened();
        Box::pin(async move {
            let open = open?;
            let name = open.name.clone();
            self.run(async move { work(open.client).await.map_err(|e| problem(&name, e)) })
                .await
        })
    }

    /// Runs `work` on the background runtime.
    fn run<'a, T: Send + 'static>(
        &'a self,
        work: impl Future<Output = Result<T, Problem>> + Send + 'static,
    ) -> impl Future<Output = Result<T, Problem>> + 'a {
        let handle = self.runtime.spawn(work);
        async move { handle.await.map_err(failed)? }
    }

    /// Opens the store at `path` and makes it the open server.
    fn adopt(&self, id: Uuid, store: Store) -> Result<Open, Problem> {
        let (name, address) = {
            let s = lock(&self.settings);
            let server = s.server(id).ok_or_else(|| failed("unknown server"))?;
            (server.name.clone(), server.address.clone())
        };
        let mut client = Client::new(&address).map_err(|e| problem(&name, e))?;
        if let Some(token) = store.token().map_err(failed)? {
            client = client.with_token(token.as_str());
        }
        let open = Open {
            id,
            name,
            store: Arc::new(store),
            client,
        };
        *lock(&self.open) = Some(open.clone());
        lock(&self.unlocked).insert(id, open.clone());
        *lock(&self.locked) = None;
        lock(&self.settings).current = Some(id);
        self.save_settings()?;
        Ok(open)
    }

    /// Replaces the open server's client (after signing in or out).
    fn set_client(&self, client: Client) {
        if let Some(open) = lock(&self.open).as_mut() {
            open.client = client.clone();
            if let Some(kept) = lock(&self.unlocked).get_mut(&open.id) {
                kept.client = client;
            }
        }
    }

    fn set_login(&self, id: Uuid, login: Option<String>) -> Result<(), Problem> {
        if let Some(s) = lock(&self.settings).server_mut(id) {
            s.login = login;
        }
        self.save_settings()
    }
}

/// Where the open server's login stands, from the server and the store.
async fn standing_of(open: Open) -> Result<Standing, Problem> {
    let p = |e| problem(&open.name, e);
    if open.store.token().map_err(failed)?.is_none() {
        return Ok(Standing::SignedOut);
    }
    let personas = match open.client.personas().await {
        Ok(list) => list,
        Err(e) if e.status() == Some(401) => return Ok(Standing::SignedOut),
        Err(e) => return Err(p(e)),
    };
    let mut states = Vec::with_capacity(personas.len());
    for persona in personas {
        let keys = persona_keys(&open, &persona).await?;
        states.push(PersonaState { persona, keys });
    }
    Ok(Standing::SignedIn {
        login: String::new(),
        personas: states,
    })
}

async fn persona_keys(open: &Open, persona: &Persona) -> Result<Keys, Problem> {
    let p = |e| problem(&open.name, e);
    let identity = keys::load_identity(&open.store, persona.id).map_err(p)?;
    let device = keys::load_device(&open.store, persona.id)
        .map_err(p)?
        .and_then(|d| d.id);
    if identity.is_some() && device.is_some() {
        return Ok(Keys::Ready);
    }
    let published = open.client.persona_keys(persona.id).await.map_err(p)?;
    Ok(match (identity, published.identity_key) {
        // Made somewhere else: this computer must be linked to it.
        (None, Some(_)) => Keys::Elsewhere,
        _ => Keys::ToSetUp,
    })
}

impl NativeBackend {
    /// [`standing_of`], with the login name the settings remember.
    fn standing_now(&self) -> impl Future<Output = Result<Standing, Problem>> + '_ {
        let open = self.opened();
        async move {
            let open = open?;
            let id = open.id;
            let mut standing = self.run(standing_of(open)).await?;
            if let Standing::SignedIn { login, .. } = &mut standing {
                *login = lock(&self.settings)
                    .server(id)
                    .and_then(|s| s.login.clone())
                    .unwrap_or_default();
            }
            Ok(standing)
        }
    }

    /// Signs in with `token` and records the login.
    async fn signed_in(&self, login: &str, token: String) -> Result<Standing, Problem> {
        let open = self.opened()?;
        open.store.set_token(Some(&token)).map_err(failed)?;
        self.set_client(open.client.clone().with_token(token));
        self.set_login(open.id, Some(login.trim().to_ascii_lowercase()))?;
        self.standing_now().await
    }
}

fn create_store(
    path: &Path,
    vault: Option<&dyn Vault>,
    passphrase: Option<&str>,
) -> Result<Store, Problem> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(failed)?;
    }
    match (passphrase, vault) {
        (Some(p), _) => Store::create(
            path,
            KeySetup::Passphrase {
                passphrase: p,
                kdf: KdfParams::DEFAULT,
            },
        )
        .map_err(failed),
        (None, Some(v)) => match Store::create(path, KeySetup::Keychain(v)) {
            Ok(s) => Ok(s),
            Err(StoreError::Keychain(_)) => Err(Problem::NeedsPassphrase),
            Err(e) => Err(failed(e)),
        },
        (None, None) => Err(Problem::NeedsPassphrase),
    }
}

impl Backend for NativeBackend {
    fn servers(&self) -> Vec<ServerEntry> {
        lock(&self.settings).servers.iter().map(entry).collect()
    }

    fn current(&self) -> Option<Uuid> {
        lock(&self.settings).current
    }

    fn check_server<'a>(&'a self, address: &'a str) -> Call<'a, ServerInfo> {
        Box::pin(async move {
            let address = normalize(address)?;
            let shown = address.clone();
            let info = self
                .run(async move {
                    let client = Client::new(&address).map_err(|e| problem(&address, e))?;
                    let info = client.instance().await.map_err(|e| problem(&address, e))?;
                    Ok(ServerInfo {
                        address,
                        domain: info.domain,
                        registration_open: info.registration_open,
                        has_rules: info.rules_updated_at.is_some(),
                        rules_to_accept: info.rules_updated_at.is_some()
                            && (info.rules_acceptance.on_join()
                                || info.rules_acceptance.on_change()),
                        software: info.software,
                        version: info.software_version,
                    })
                })
                .await;
            // A wrong address and an offline server look alike from here.
            info.map_err(|e| match e {
                Problem::Unreachable(_) | Problem::Failed { .. } => Problem::Unreachable(shown),
                other => other,
            })
        })
    }

    fn add_server<'a>(
        &'a self,
        address: &'a str,
        passphrase: Option<&'a str>,
    ) -> Call<'a, ServerEntry> {
        Box::pin(async move {
            let info = self.check_server(address).await?;
            if lock(&self.settings)
                .servers
                .iter()
                .any(|s| s.address == info.address)
            {
                return Err(Problem::Refused(format!(
                    "{} is already in your list of servers.",
                    info.domain
                )));
            }
            if let Some(p) = passphrase {
                check_passphrase(p)?;
            }
            let id = Uuid::new_v4();
            let path = self.store_path(id);
            let store = create_store(&path, self.vault.as_deref(), passphrase)?;
            store.set_server(&info.address).map_err(failed)?;
            let server = Server {
                id,
                address: info.address,
                name: info.domain,
                login: None,
            };
            lock(&self.settings).servers.push(server.clone());
            self.adopt(id, store)?;
            Ok(entry(&server))
        })
    }

    fn open_server(&self, id: Uuid) -> Call<'_, Standing> {
        Box::pin(async move {
            if lock(&self.open).as_ref().is_some_and(|o| o.id == id) {
                return self.standing_now().await;
            }
            let kept = lock(&self.unlocked).get(&id).cloned();
            if let Some(open) = kept {
                *lock(&self.open) = Some(open);
                *lock(&self.locked) = None;
                lock(&self.settings).current = Some(id);
                self.save_settings()?;
                return self.standing_now().await;
            }
            let path = self.store_path(id);
            if !path.exists() {
                return Err(failed(format!("no store at {}", path.display())));
            }
            match Store::key_source(&path).map_err(failed)? {
                KeySource::Keychain => {
                    let vault = self.vault.clone().ok_or(Problem::NeedsPassphrase)?;
                    let store = self
                        .run(async move {
                            Store::open(&path, Unlock::Keychain(vault.as_ref())).map_err(failed)
                        })
                        .await?;
                    self.adopt(id, store)?;
                    self.standing_now().await
                }
                KeySource::Passphrase => {
                    *lock(&self.open) = None;
                    *lock(&self.locked) = Some(Locked { id, path });
                    lock(&self.settings).current = Some(id);
                    self.save_settings()?;
                    Ok(Standing::Locked)
                }
            }
        })
    }

    fn unlock<'a>(&'a self, passphrase: &'a str) -> Call<'a, Standing> {
        Box::pin(async move {
            let locked = lock(&self.locked)
                .clone()
                .ok_or_else(|| failed("no store is waiting to be unlocked"))?;
            let passphrase = passphrase.to_owned();
            let path = locked.path.clone();
            let store = self
                .run(async move {
                    Store::open(&path, Unlock::Passphrase(&passphrase)).map_err(|e| match e {
                        StoreError::WrongKey => {
                            Problem::Refused("That isn't the passphrase.".into())
                        }
                        e => failed(e),
                    })
                })
                .await?;
            self.adopt(locked.id, store)?;
            self.standing_now().await
        })
    }

    fn remove_server(&self, id: Uuid) -> Call<'_, ()> {
        Box::pin(async move {
            if lock(&self.open).as_ref().is_some_and(|o| o.id == id) {
                *lock(&self.open) = None;
            }
            lock(&self.unlocked).remove(&id);
            if lock(&self.locked).as_ref().is_some_and(|l| l.id == id) {
                *lock(&self.locked) = None;
            }
            let path = self.store_path(id);
            let vault = self.vault.clone();
            self.run(async move {
                Store::destroy(&path, vault.as_deref()).map_err(|e| match e {
                    StoreError::InUse => busy(),
                    e => failed(e),
                })
            })
            .await?;
            {
                let mut s = lock(&self.settings);
                s.servers.retain(|s| s.id != id);
                if s.current == Some(id) {
                    s.current = None;
                }
            }
            self.save_settings()
        })
    }

    fn rename_server(&self, id: Uuid, name: &str) -> Result<(), Problem> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 60 {
            return Err(Problem::Refused("Use 1 to 60 characters.".into()));
        }
        lock(&self.settings)
            .server_mut(id)
            .ok_or_else(|| failed("unknown server"))?
            .name = name.to_owned();
        if let Some(open) = lock(&self.open).as_mut().filter(|o| o.id == id) {
            open.name = name.to_owned();
        }
        if let Some(kept) = lock(&self.unlocked).get_mut(&id) {
            kept.name = name.to_owned();
        }
        self.save_settings()
    }

    fn keychain_available(&self) -> bool {
        self.vault.is_some()
    }

    fn protection(&self) -> Option<Protection> {
        let open = lock(&self.open).clone()?;
        match open.store.status().ok()?.key_source? {
            KeySource::Keychain => Some(Protection::Keychain),
            KeySource::Passphrase => Some(Protection::Passphrase),
        }
    }

    fn set_protection<'a>(&'a self, passphrase: Option<&'a str>) -> Call<'a, ()> {
        Box::pin(async move {
            if let Some(p) = passphrase {
                check_passphrase(p)?;
            } else if self.vault.is_none() {
                return Err(Problem::NeedsPassphrase);
            }
            // Changing the key needs the only handle on the store.
            let open = lock(&self.open)
                .take()
                .ok_or_else(|| failed("no server is open"))?;
            lock(&self.unlocked).remove(&open.id);
            let Open {
                id,
                name,
                store,
                client,
            } = open;
            let store = match Arc::try_unwrap(store) {
                Ok(s) => s,
                Err(store) => {
                    let open = Open {
                        id,
                        name,
                        store,
                        client,
                    };
                    lock(&self.unlocked).insert(id, open.clone());
                    *lock(&self.open) = Some(open);
                    return Err(busy());
                }
            };
            let vault = self.vault.clone();
            let passphrase = passphrase.map(str::to_owned);
            // The task always hands the store back, changed or not.
            let (store, result) = self
                .run(async move {
                    let mut store = store;
                    let old = match store.status().map(|s| s.key_source) {
                        Ok(Some(KeySource::Keychain)) => vault.as_deref(),
                        _ => None,
                    };
                    let setup = match (&passphrase, vault.as_deref()) {
                        (Some(p), _) => KeySetup::Passphrase {
                            passphrase: p,
                            kdf: KdfParams::DEFAULT,
                        },
                        (None, Some(v)) => KeySetup::Keychain(v),
                        (None, None) => return Ok((store, Err(Problem::NeedsPassphrase))),
                    };
                    let result = store.change_key(setup, old).map_err(failed);
                    Ok((store, result))
                })
                .await?;
            let open = Open {
                id,
                name,
                store: Arc::new(store),
                client,
            };
            lock(&self.unlocked).insert(id, open.clone());
            *lock(&self.open) = Some(open);
            result
        })
    }

    fn standing(&self) -> Call<'_, Standing> {
        Box::pin(async move {
            if lock(&self.locked).is_some() {
                return Ok(Standing::Locked);
            }
            self.standing_now().await
        })
    }

    fn register<'a>(
        &'a self,
        login: &'a str,
        password: &'a str,
        accept_rules: Option<u32>,
    ) -> Call<'a, Standing> {
        Box::pin(async move {
            let open = self.opened()?;
            let body = Registration {
                username: login.to_owned(),
                password: password.to_owned(),
                accept_rules,
            };
            let name = open.name.clone();
            let client = open.client.clone();
            let token = self
                .run(async move {
                    client
                        .register_with(&body)
                        .await
                        .map(|t| t.token)
                        .map_err(|e| problem(&name, e))
                })
                .await?;
            self.signed_in(login, token).await
        })
    }

    fn sign_in<'a>(&'a self, login: &'a str, password: &'a str) -> Call<'a, Standing> {
        Box::pin(async move {
            let open = self.opened()?;
            let creds = Credentials {
                username: login.to_owned(),
                password: password.to_owned(),
            };
            let name = open.name.clone();
            let client = open.client.clone();
            let token = self
                .run(async move {
                    client
                        .login(&creds)
                        .await
                        .map(|t| t.token)
                        .map_err(|e| match e {
                            CoreError::Api(p) if p.status == 401 => {
                                Problem::Refused("That login name and password don't match.".into())
                            }
                            e => problem(&name, e),
                        })
                })
                .await?;
            self.signed_in(login, token).await
        })
    }

    fn sign_out(&self) -> Call<'_, ()> {
        Box::pin(async move {
            let open = self.opened()?;
            let client = open.client.clone();
            // Best effort: the token is forgotten here either way.
            let _ = self
                .run(async move { client.logout().await.map_err(failed) })
                .await;
            open.store.set_token(None).map_err(failed)?;
            let address = lock(&self.settings)
                .server(open.id)
                .map(|s| s.address.clone())
                .unwrap_or_default();
            self.set_client(Client::new(&address).map_err(failed)?);
            Ok(())
        })
    }

    fn rules(&self) -> Call<'_, Option<ServerRules>> {
        Box::pin(async move {
            let open = self.opened()?;
            self.run(async move {
                match open.client.rules().await {
                    Ok(r) => Ok(Some(ServerRules {
                        markdown: r.body,
                        version: r.version,
                        must_accept: r.must_accept,
                    })),
                    // No rules yet.
                    Err(e) if e.status() == Some(404) => Ok(None),
                    Err(e) => Err(problem(&open.name, e)),
                }
            })
            .await
        })
    }

    fn accept_rules(&self, version: u32) -> Call<'_, ()> {
        Box::pin(async move {
            let open = self.opened()?;
            self.run(async move {
                open.client
                    .accept_rules(version)
                    .await
                    .map_err(|e| match e {
                        // Changed again while being read.
                        CoreError::Api(p) if p.status == 409 => Problem::RulesNotAccepted(
                            "The rules changed again while you were reading. Read the new version."
                                .into(),
                        ),
                        e => problem(&open.name, e),
                    })
            })
            .await
        })
    }

    fn create_persona<'a>(&'a self, handle: &'a str, display_name: &'a str) -> Call<'a, Persona> {
        Box::pin(async move {
            let open = self.opened()?;
            let body = CreatePersona {
                handle: handle.trim().to_owned(),
                display_name: Some(display_name.trim().to_owned()).filter(|d| !d.is_empty()),
                summary: None,
            };
            self.run(async move {
                open.client
                    .create_persona(&body)
                    .await
                    .map_err(|e| problem(&open.name, e))
            })
            .await
        })
    }

    fn update_persona<'a>(
        &'a self,
        id: Uuid,
        display_name: &'a str,
        summary: &'a str,
    ) -> Call<'a, Persona> {
        Box::pin(async move {
            let open = self.opened()?;
            let body = UpdatePersona {
                display_name: Some(display_name.trim().to_owned()),
                summary: Some(summary.trim().to_owned()),
            };
            self.run(async move {
                open.client
                    .update_persona(id, &body)
                    .await
                    .map_err(|e| problem(&open.name, e))
            })
            .await
        })
    }

    fn delete_persona(&self, id: Uuid) -> Call<'_, ()> {
        Box::pin(async move {
            let open = self.opened()?;
            self.run(async move {
                open.client
                    .delete_persona(id)
                    .await
                    .map_err(|e| problem(&open.name, e))
            })
            .await
        })
    }

    fn prepare<'a>(&'a self, persona: Uuid, progress: &'a dyn Fn(Step)) -> Call<'a, ()> {
        Box::pin(async move {
            let open = self.opened()?;
            let o = open.clone();
            let found = self
                .run(async move {
                    let list = o.client.personas().await.map_err(|e| problem(&o.name, e))?;
                    Ok(list.into_iter().find(|p| p.id == persona))
                })
                .await?
                .ok_or_else(|| Problem::Refused("That persona is gone.".into()))?;
            progress(Step::IdentityKey);
            let (o, p) = (open.clone(), found.clone());
            self.run(async move {
                keys::init(&o.store, &o.client, &p, false)
                    .await
                    .map(|_| ())
                    .map_err(|e| match e {
                        // Made on another device: linking is the way (3d).
                        CoreError::Keys(_) => Problem::Refused(
                            "This persona's keys are on another of your devices. Link this computer to it, or restore a backup.".into(),
                        ),
                        e => problem(&o.name, e),
                    })
            })
            .await?;
            progress(Step::ThisComputer);
            let o = open.clone();
            self.run(async move {
                devices::announce(&o.store, &o.client)
                    .await
                    .map_err(|e| problem(&o.name, e))
            })
            .await?;
            progress(Step::KeyPackages);
            let o = open;
            self.run(async move {
                keys::publish(&o.store, &o.client, &found, keys::POOL_TARGET)
                    .await
                    .map(|_| ())
                    .map_err(|e| problem(&o.name, e))
            })
            .await
        })
    }

    fn upkeep(&self) -> Call<'_, Option<String>> {
        Box::pin(async move {
            let open = self.opened()?;
            if open.store.token().map_err(failed)?.is_none() {
                return Ok(None);
            }
            let name = open.name.clone();
            let report = self
                .run(async move {
                    devices::maintain(&open.store, &open.client)
                        .await
                        .map_err(|e| problem(&name, e))
                })
                .await;
            let (line, tell) = match &report {
                Ok(r) if r.caught_up.removed => (
                    "removed from the login's devices".to_owned(),
                    Some("This computer was removed from your devices. Link it again to keep writing here.".to_owned()),
                ),
                Ok(r) if r.caught_up.stale => (
                    "fell behind the device group".to_owned(),
                    Some("This computer was away too long to catch up. Link it again from another device.".to_owned()),
                ),
                Ok(r) => (
                    format!(
                        "ok: {} refilled, {} history problem",
                        r.refilled.len(),
                        r.history_problem.as_deref().unwrap_or("no")
                    ),
                    None,
                ),
                Err(e) => (format!("error: {e} {}", e.detail().unwrap_or("")), None),
            };
            {
                let mut log = lock(&self.upkeep_log);
                log.push(line);
                let excess = log.len().saturating_sub(UPKEEP_LOG);
                log.drain(..excess);
            }
            report.map(|_| tell)
        })
    }

    fn next_notice(&self) -> Call<'_, ()> {
        Box::pin(async move {
            let open = self.opened()?;
            self.run(async move {
                let p = |e| problem(&open.name, e);
                let mut stream = open.client.events().await.map_err(p)?;
                match stream.next().await.map_err(p)? {
                    Some(_) => Ok(()),
                    // The server closed the stream: wait before reopening.
                    None => Err(Problem::Unreachable(open.name.clone())),
                }
            })
            .await
        })
    }

    fn account(&self) -> Call<'_, AccountSettings> {
        self.call(|c| async move { c.account().await })
    }

    fn prompts(&self, query: PromptQuery) -> Call<'_, PromptPage> {
        self.call(|c| async move { c.prompts(&query).await })
    }

    fn prompt(&self, id: Uuid) -> Call<'_, Prompt> {
        self.call(move |c| async move { c.prompt(id).await })
    }

    fn create_prompt(&self, body: CreatePrompt) -> Call<'_, Prompt> {
        self.call(|c| async move { c.create_prompt(&body).await })
    }

    fn update_prompt(&self, id: Uuid, body: UpdatePrompt) -> Call<'_, Prompt> {
        self.call(move |c| async move { c.update_prompt(id, &body).await })
    }

    fn delete_prompt(&self, id: Uuid) -> Call<'_, ()> {
        self.call(move |c| async move { c.delete_prompt(id).await })
    }

    fn prompt_link(&self, id: Uuid) -> Option<String> {
        let open = lock(&self.open).clone()?;
        let address = lock(&self.settings).server(open.id)?.address.clone();
        Some(format!("{address}/p/{id}"))
    }

    fn tag_types(&self) -> Call<'_, Vec<TagNamespace>> {
        self.call(|c| async move { c.tag_namespaces().await })
    }

    fn tag_policy(&self) -> Call<'_, TagPolicy> {
        self.call(|c| async move { c.instance().await.map(|i| i.tag_policy) })
    }

    fn tag_page<'a>(&'a self, namespace: &'a str, query: TagPageQuery) -> Call<'a, TagPage> {
        let namespace = namespace.to_owned();
        self.call(move |c| async move { c.tag_page(&namespace, &query).await })
    }

    fn find_tags(&self, query: TagQuery) -> Call<'_, Vec<Tag>> {
        self.call(|c| async move { c.tags(&query).await })
    }

    fn frequent_tags(&self, persona: Option<Uuid>) -> Call<'_, Vec<Tag>> {
        self.call(move |c| async move { c.frequent_tags(persona).await })
    }

    fn report(&self, body: CreateReport) -> Call<'_, ()> {
        self.call(|c| async move { c.report(&body).await })
    }

    fn notices(&self, before: Option<i64>) -> Call<'_, Vec<AccountNotice>> {
        self.call(move |c| async move { c.notices(before).await })
    }

    fn mark_notices_read(&self) -> Call<'_, ()> {
        self.call(|c| async move { c.mark_notices_read().await })
    }

    fn board_settings(&self) -> Call<'_, BoardSettings> {
        Box::pin(async move {
            let open = self.opened()?;
            self.run(async move {
                open.client
                    .account()
                    .await
                    .map(|a| BoardSettings {
                        maturity_max: a.maturity_max,
                    })
                    .map_err(|e| problem(&open.name, e))
            })
            .await
        })
    }

    fn set_board_settings(&self, settings: BoardSettings) -> Call<'_, BoardSettings> {
        Box::pin(async move {
            let open = self.opened()?;
            self.run(async move {
                open.client
                    .update_account(&UpdateAccountSettings {
                        maturity_max: Some(settings.maturity_max),
                        ..Default::default()
                    })
                    .await
                    .map(|a| BoardSettings {
                        maturity_max: a.maturity_max,
                    })
                    .map_err(|e| problem(&open.name, e))
            })
            .await
        })
    }

    fn app_settings(&self) -> AppSettings {
        let folder = self.stores_folder();
        let s = lock(&self.settings);
        AppSettings {
            theme: match s.theme {
                settings::Theme::System => Theme::System,
                settings::Theme::Light => Theme::Light,
                settings::Theme::Dark => Theme::Dark,
            },
            text_size: s.text_size,
            advanced: s.advanced,
            stores_folder: folder.display().to_string(),
        }
    }

    fn set_app_settings(&self, new: AppSettings) -> Result<(), Problem> {
        {
            let mut s = lock(&self.settings);
            s.theme = match new.theme {
                Theme::System => settings::Theme::System,
                Theme::Light => settings::Theme::Light,
                Theme::Dark => settings::Theme::Dark,
            };
            s.text_size = new.text_size.clamp(80, 160);
            s.advanced = new.advanced;
        }
        self.save_settings()
    }

    fn move_stores<'a>(&'a self, folder: &'a str) -> Call<'a, ()> {
        Box::pin(async move {
            let to = PathBuf::from(folder.trim());
            if !to.is_absolute() {
                return Err(Problem::Refused("Choose a full folder path.".into()));
            }
            let from = self.stores_folder();
            if to == from {
                return Ok(());
            }
            let ids: Vec<Uuid> = lock(&self.settings).servers.iter().map(|s| s.id).collect();
            for id in &ids {
                let target = to.join(format!("{id}.quill"));
                if target.exists() {
                    return Err(Problem::Refused(format!(
                        "{} already holds a store for one of your servers.",
                        to.display()
                    )));
                }
            }
            // Close the stores while their files move.
            let current = lock(&self.open).take().map(|o| o.id);
            lock(&self.unlocked).clear();
            std::fs::create_dir_all(&to).map_err(failed)?;
            for id in &ids {
                let source = from.join(format!("{id}.quill"));
                if !source.exists() {
                    continue;
                }
                let target = to.join(format!("{id}.quill"));
                if std::fs::rename(&source, &target).is_err() {
                    // Another disk: copy, then remove the original.
                    std::fs::copy(&source, &target).map_err(failed)?;
                    std::fs::remove_file(&source).map_err(failed)?;
                }
            }
            lock(&self.settings).stores_folder = Some(to);
            self.save_settings()?;
            if let Some(id) = current {
                self.open_server(id).await?;
            }
            Ok(())
        })
    }

    fn sleep(&self, ms: u64) -> std::pin::Pin<Box<dyn Future<Output = ()> + '_>> {
        let timer = self
            .runtime
            .spawn(tokio::time::sleep(std::time::Duration::from_millis(ms)));
        Box::pin(async move {
            let _ = timer.await;
        })
    }

    fn details(&self) -> Call<'_, Details> {
        Box::pin(async move {
            let mut rows: Vec<(String, String)> = Vec::new();
            let folder = self.stores_folder();
            rows.push(("Stores folder".into(), folder.display().to_string()));
            rows.push((
                "Settings file".into(),
                self.paths.settings.display().to_string(),
            ));
            let Ok(open) = self.opened() else {
                return Ok(Details { rows });
            };
            rows.push(("Server id (this app's)".into(), open.id.to_string()));
            if let Ok(status) = open.store.status() {
                if let Some(path) = status.path {
                    rows.push(("Store".into(), path.display().to_string()));
                }
                rows.push((
                    "Store key".into(),
                    match status.key_source {
                        Some(KeySource::Keychain) => "system keychain",
                        Some(KeySource::Passphrase) => "passphrase",
                        None => "in memory",
                    }
                    .into(),
                ));
                rows.push(("Store size".into(), format!("{} bytes", status.file_bytes)));
            }
            let o = open.clone();
            let extra = self
                .run(async move {
                    let mut rows = Vec::new();
                    if let Ok(info) = o.client.instance().await {
                        rows.push((
                            "Server software".into(),
                            format!("{} {}", info.software, info.software_version),
                        ));
                        rows.push((
                            "Message limit".into(),
                            format!("{} bytes", info.limits.max_message_bytes),
                        ));
                    }
                    if let Ok(personas) = o.client.personas().await {
                        for p in personas {
                            rows.push((format!("@{} id", p.handle), p.id.to_string()));
                            if let Ok(Some(identity)) = keys::load_identity(&o.store, p.id) {
                                rows.push((
                                    format!("@{} identity key", p.handle),
                                    identity.fingerprint(),
                                ));
                            }
                            let device = keys::load_device(&o.store, p.id).ok().flatten();
                            if let Some(d) = &device {
                                rows.push((
                                    format!("@{} device key", p.handle),
                                    quill_certs::fingerprint(&d.public()),
                                ));
                            }
                            if let Some(dev) = device.and_then(|d| d.id) {
                                rows.push((format!("@{} device", p.handle), dev.to_string()));
                                if let Ok(s) = o.client.key_package_status(p.id, dev).await {
                                    rows.push((
                                        format!("@{} KeyPackages", p.handle),
                                        s.available.to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    Ok(rows)
                })
                .await
                .unwrap_or_default();
            rows.extend(extra);
            for (i, line) in lock(&self.upkeep_log).iter().enumerate() {
                rows.push((format!("Upkeep {}", i + 1), line.clone()));
            }
            Ok(Details { rows })
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn typed_addresses_get_the_right_scheme() {
        assert_eq!(
            normalize("writers.example/").unwrap(),
            "https://writers.example"
        );
        assert_eq!(
            normalize("localhost:8088").unwrap(),
            "http://localhost:8088"
        );
        assert_eq!(normalize("127.0.0.1").unwrap(), "http://127.0.0.1");
        assert_eq!(normalize("[::1]:8080").unwrap(), "http://[::1]:8080");
        assert_eq!(
            normalize("https://localhost:8088").unwrap(),
            "https://localhost:8088"
        );
        // Only this computer gets plain http.
        assert_eq!(
            normalize("localhost.example").unwrap(),
            "https://localhost.example"
        );
        assert!(normalize("http://writers.example").is_err());
        assert!(normalize(" ").is_err());
    }
}
