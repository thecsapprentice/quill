//! A backend held in memory, for the views' tests: every call answers at
//! once from what the test set up.

use std::{cell::RefCell, future::Future, pin::Pin};

use quill_proto::board::Maturity;
use uuid::Uuid;

use crate::backend::{
    AppSettings, Backend, BoardSettings, Call, Details, Keys, Persona, PersonaState, Problem,
    Protection, ServerEntry, ServerInfo, ServerRules, Standing, Step, Theme,
};

pub struct Fake {
    pub servers: RefCell<Vec<ServerEntry>>,
    pub current: RefCell<Option<Uuid>>,
    pub standing: RefCell<Standing>,
    pub settings: RefCell<AppSettings>,
}

pub fn persona(handle: &str, keys: Keys) -> PersonaState {
    PersonaState {
        persona: Persona {
            id: Uuid::new_v4(),
            handle: handle.into(),
            display_name: Some(format!("{handle} (display)")),
            summary: Some("Writes about foxes.".into()),
            actor_uri: format!("https://writers.example/actors/{handle}"),
            created_at: time::OffsetDateTime::UNIX_EPOCH,
        },
        keys,
    }
}

impl Fake {
    /// One server, signed in as `ink` with `personas`.
    pub fn signed_in(personas: Vec<PersonaState>) -> Self {
        let id = Uuid::new_v4();
        Self {
            servers: RefCell::new(vec![ServerEntry {
                id,
                address: "https://writers.example".into(),
                name: "writers.example".into(),
                login: Some("ink".into()),
            }]),
            current: RefCell::new(Some(id)),
            standing: RefCell::new(Standing::SignedIn {
                login: "ink".into(),
                personas,
            }),
            settings: RefCell::new(AppSettings {
                theme: Theme::System,
                text_size: 100,
                advanced: false,
                stores_folder: "/home/ink/.local/share/quill-gui/stores".into(),
            }),
        }
    }

    pub fn empty() -> Self {
        let f = Self::signed_in(Vec::new());
        f.servers.borrow_mut().clear();
        *f.current.borrow_mut() = None;
        *f.standing.borrow_mut() = Standing::SignedOut;
        f
    }
}

fn ready<'a, T: 'a>(v: Result<T, Problem>) -> Call<'a, T> {
    Box::pin(async move { v })
}

impl Backend for Fake {
    fn servers(&self) -> Vec<ServerEntry> {
        self.servers.borrow().clone()
    }
    fn current(&self) -> Option<Uuid> {
        *self.current.borrow()
    }
    fn check_server<'a>(&'a self, address: &'a str) -> Call<'a, ServerInfo> {
        ready(Ok(ServerInfo {
            address: address.into(),
            domain: address.into(),
            registration_open: true,
            has_rules: true,
            rules_to_accept: true,
            software: "quill".into(),
            version: "0.0.0".into(),
        }))
    }
    fn add_server<'a>(&'a self, _: &'a str, _: Option<&'a str>) -> Call<'a, ServerEntry> {
        ready(Err(Problem::NeedsPassphrase))
    }
    fn open_server(&self, _: Uuid) -> Call<'_, Standing> {
        ready(Ok(self.standing.borrow().clone()))
    }
    fn unlock<'a>(&'a self, _: &'a str) -> Call<'a, Standing> {
        ready(Ok(self.standing.borrow().clone()))
    }
    fn remove_server(&self, _: Uuid) -> Call<'_, ()> {
        ready(Ok(()))
    }
    fn rename_server(&self, _: Uuid, _: &str) -> Result<(), Problem> {
        Ok(())
    }
    fn keychain_available(&self) -> bool {
        true
    }
    fn protection(&self) -> Option<Protection> {
        Some(Protection::Keychain)
    }
    fn set_protection<'a>(&'a self, _: Option<&'a str>) -> Call<'a, ()> {
        ready(Ok(()))
    }
    fn standing(&self) -> Call<'_, Standing> {
        ready(Ok(self.standing.borrow().clone()))
    }
    fn register<'a>(&'a self, _: &'a str, _: &'a str, _: Option<u32>) -> Call<'a, Standing> {
        ready(Err(Problem::Taken))
    }
    fn sign_in<'a>(&'a self, _: &'a str, _: &'a str) -> Call<'a, Standing> {
        ready(Ok(self.standing.borrow().clone()))
    }
    fn sign_out(&self) -> Call<'_, ()> {
        ready(Ok(()))
    }
    fn rules(&self) -> Call<'_, Option<ServerRules>> {
        ready(Ok(Some(ServerRules {
            markdown: "# Be kind\n\nNo <b>spam</b>.".into(),
            version: 2,
            must_accept: true,
        })))
    }
    fn accept_rules(&self, _: u32) -> Call<'_, ()> {
        ready(Ok(()))
    }
    fn create_persona<'a>(&'a self, _: &'a str, _: &'a str) -> Call<'a, Persona> {
        ready(Err(Problem::Taken))
    }
    fn update_persona<'a>(&'a self, _: Uuid, _: &'a str, _: &'a str) -> Call<'a, Persona> {
        ready(Err(Problem::Refused("no".into())))
    }
    fn delete_persona(&self, _: Uuid) -> Call<'_, ()> {
        ready(Ok(()))
    }
    fn prepare<'a>(&'a self, _: Uuid, progress: &'a dyn Fn(Step)) -> Call<'a, ()> {
        progress(Step::IdentityKey);
        ready(Ok(()))
    }
    fn upkeep(&self) -> Call<'_, Option<String>> {
        ready(Ok(None))
    }
    fn next_notice(&self) -> Call<'_, ()> {
        Box::pin(std::future::pending())
    }
    fn board_settings(&self) -> Call<'_, BoardSettings> {
        ready(Ok(BoardSettings {
            maturity_max: Maturity::Teen,
        }))
    }
    fn set_board_settings(&self, s: BoardSettings) -> Call<'_, BoardSettings> {
        ready(Ok(s))
    }
    fn app_settings(&self) -> AppSettings {
        self.settings.borrow().clone()
    }
    fn set_app_settings(&self, s: AppSettings) -> Result<(), Problem> {
        *self.settings.borrow_mut() = s;
        Ok(())
    }
    fn move_stores<'a>(&'a self, _: &'a str) -> Call<'a, ()> {
        ready(Ok(()))
    }
    fn details(&self) -> Call<'_, Details> {
        ready(Ok(Details {
            rows: vec![("Store".into(), "/tmp/store.quill".into())],
        }))
    }
    fn sleep(&self, _: u64) -> Pin<Box<dyn Future<Output = ()> + '_>> {
        Box::pin(std::future::pending())
    }
}
