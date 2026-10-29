//! The app's state and how it moves, kept apart from the views so the
//! end-to-end tests can drive the same steps against a real server.
//!
//! Views make the backend call, then hand its result to one of [`Model`]'s
//! methods; nothing here awaits. Results for a server that's no longer the
//! one in view are dropped, so switching quickly never shows stale data.

use uuid::Uuid;

use crate::backend::{
    AppSettings, Backend, Keys, Persona, PersonaState, Problem, ServerEntry, ServerRules, Standing,
};

/// What fills the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Opening the last server, briefly.
    Starting,
    /// Which server (first run, or adding one).
    Server,
    /// The store waits for its passphrase.
    Unlock,
    /// New here, or have a login.
    Account,
    /// The first persona, pushed but skippable.
    FirstPersona,
    /// Getting a persona ready, as progress.
    Preparing { persona: Uuid },
    /// Signed in where the personas' keys are on another device.
    Elsewhere,
    /// The app's frame and its sections.
    Frame,
}

/// The frame's sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Home,
    Board,
    Requests,
    Sessions,
    Personas,
    Settings,
    /// The server rules: reached from Settings and the "rules changed"
    /// banner, not the sidebar.
    Rules,
}

impl Section {
    pub const ALL: [Section; 6] = [
        Section::Home,
        Section::Board,
        Section::Requests,
        Section::Sessions,
        Section::Personas,
        Section::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Home => "Home",
            Section::Board => "Board",
            Section::Requests => "Requests",
            Section::Sessions => "Sessions",
            Section::Personas => "Personas",
            Section::Settings => "Settings",
            Section::Rules => "Server rules",
        }
    }
}

/// The status line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Connecting,
    Connected,
    /// Can't reach the server; the app keeps trying.
    Offline,
    SignedOut,
    Locked,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    pub screen: Screen,
    pub section: Section,
    pub servers: Vec<ServerEntry>,
    /// The server in view.
    pub current: Option<Uuid>,
    /// Where its login stands, once known.
    pub standing: Option<Standing>,
    /// The persona in use.
    pub persona: Option<Uuid>,
    pub status: Status,
    /// What went wrong on the screen in view.
    pub problem: Option<Problem>,
    /// What upkeep says needs the writer.
    pub notice: Option<String>,
    /// The server's rules, once fetched (`None` inside: it has none).
    pub rules: Option<Option<ServerRules>>,
    pub settings: AppSettings,
    /// Adding a server: the one to go back to if the writer cancels.
    pub adding: Option<Option<Uuid>>,
    /// Signed in (or registered) during this run: the first persona is
    /// pushed only then.
    pub fresh_login: bool,
    pub persona_skipped: bool,
    /// The writer chose to get a persona ready later.
    pub prepare_later: bool,
    pub elsewhere_seen: bool,
    pub backup_nudge_dismissed: bool,
    /// Bumped to restart the upkeep loop now (the "Try now" button).
    pub upkeep_round: u32,
}

impl Model {
    /// The model as the app opens, from the backend's settings.
    pub fn load(backend: &dyn Backend) -> Self {
        let servers = backend.servers();
        let current = backend
            .current()
            .filter(|id| servers.iter().any(|s| s.id == *id))
            .or_else(|| servers.first().map(|s| s.id));
        Self {
            screen: if current.is_some() {
                Screen::Starting
            } else {
                Screen::Server
            },
            section: Section::Home,
            servers,
            current,
            standing: None,
            persona: None,
            status: Status::Connecting,
            problem: None,
            notice: None,
            rules: None,
            settings: backend.app_settings(),
            adding: None,
            fresh_login: false,
            persona_skipped: false,
            prepare_later: false,
            elsewhere_seen: false,
            backup_nudge_dismissed: false,
            upkeep_round: 0,
        }
    }

    pub fn server(&self) -> Option<&ServerEntry> {
        self.servers.iter().find(|s| Some(s.id) == self.current)
    }

    /// The server's name, for sentences.
    pub fn server_name(&self) -> String {
        self.server()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "this server".into())
    }

    pub fn personas(&self) -> &[PersonaState] {
        match &self.standing {
            Some(Standing::SignedIn { personas, .. }) => personas,
            _ => &[],
        }
    }

    pub fn persona_in_use(&self) -> Option<&PersonaState> {
        self.personas()
            .iter()
            .find(|p| Some(p.persona.id) == self.persona)
    }

    pub fn signed_in(&self) -> bool {
        matches!(self.standing, Some(Standing::SignedIn { .. }))
    }

    /// Whether this login has been signed in before (so the frame has
    /// something to show even while signed out or offline).
    fn login_known(&self) -> bool {
        self.server().is_some_and(|s| s.login.is_some())
    }

    pub fn refresh_servers(&mut self, backend: &dyn Backend) {
        self.servers = backend.servers();
    }

    /// Switching to (or first opening) server `id`. The frame stays up
    /// while it opens, so switching feels instant.
    pub fn opening(&mut self, id: Uuid) {
        if self.current != Some(id) {
            self.persona = None;
            self.notice = None;
            self.rules = None;
        }
        self.current = Some(id);
        self.standing = None;
        self.status = Status::Connecting;
        self.problem = None;
        self.prepare_later = false;
        self.elsewhere_seen = false;
        if self.screen != Screen::Starting && self.login_known() {
            self.screen = Screen::Frame;
        }
    }

    /// Where server `id`'s login stands now, or why that isn't known.
    pub fn opened(&mut self, id: Uuid, result: Result<Standing, Problem>) {
        if self.current != Some(id) {
            return;
        }
        match result {
            Ok(standing) => {
                self.problem = None;
                self.set_standing(standing);
            }
            Err(Problem::SignedOut) => self.set_standing(Standing::SignedOut),
            Err(Problem::Unreachable(name)) => {
                // Not known now (even if it was, say, locked a moment ago):
                // the upkeep loop loads it once the server answers.
                self.standing = None;
                self.status = Status::Offline;
                if self.login_known() {
                    self.screen = Screen::Frame;
                } else {
                    // First run, interrupted: the account step says it.
                    self.problem = Some(Problem::Unreachable(name));
                    self.screen = Screen::Account;
                }
            }
            Err(other) => {
                self.problem = Some(other);
                if self.screen == Screen::Starting {
                    self.screen = Screen::Frame;
                }
            }
        }
    }

    /// After registering or signing in.
    pub fn signed_in_now(&mut self, backend: &dyn Backend, id: Uuid, standing: Standing) {
        self.refresh_servers(backend);
        self.fresh_login = true;
        self.opened(id, Ok(standing));
    }

    fn set_standing(&mut self, standing: Standing) {
        self.status = match standing {
            Standing::Locked => Status::Locked,
            Standing::SignedOut => Status::SignedOut,
            Standing::SignedIn { .. } => Status::Connected,
        };
        self.standing = Some(standing);
        self.pick_persona();
        self.route();
    }

    /// Keeps the persona in use if it's still here, else picks the first
    /// one ready on this computer.
    fn pick_persona(&mut self) {
        let personas = self.personas();
        let keep = personas
            .iter()
            .any(|p| Some(p.persona.id) == self.persona && p.keys == Keys::Ready);
        if !keep {
            self.persona = personas
                .iter()
                .find(|p| p.keys == Keys::Ready)
                .map(|p| p.persona.id);
        }
    }

    /// Decides the screen from where the login stands.
    fn route(&mut self) {
        self.screen = match &self.standing {
            None => self.screen,
            Some(Standing::Locked) => Screen::Unlock,
            Some(Standing::SignedOut) => {
                if self.adding.is_some() || !self.login_known() || self.screen != Screen::Frame {
                    Screen::Account
                } else {
                    // The status line offers the way back in.
                    Screen::Frame
                }
            }
            Some(Standing::SignedIn { personas, .. }) => {
                let to_set_up = personas.iter().find(|p| p.keys == Keys::ToSetUp);
                if let (Some(p), false) = (to_set_up, self.prepare_later) {
                    Screen::Preparing {
                        persona: p.persona.id,
                    }
                } else if personas.is_empty() && self.fresh_login && !self.persona_skipped {
                    Screen::FirstPersona
                } else if !personas.is_empty()
                    && personas.iter().all(|p| p.keys == Keys::Elsewhere)
                    && !self.elsewhere_seen
                {
                    Screen::Elsewhere
                } else {
                    self.adding = None;
                    Screen::Frame
                }
            }
        };
    }

    pub fn skip_persona(&mut self) {
        self.persona_skipped = true;
        self.route();
    }

    pub fn persona_created(&mut self, persona: Persona) {
        let id = persona.id;
        if let Some(Standing::SignedIn { personas, .. }) = &mut self.standing {
            personas.push(PersonaState {
                persona,
                keys: Keys::ToSetUp,
            });
        }
        self.problem = None;
        self.prepare_later = false;
        self.screen = Screen::Preparing { persona: id };
    }

    /// Getting `persona` ready finished, or stopped with a problem (the
    /// screen stays, offering to try again).
    pub fn prepared(&mut self, persona: Uuid, result: Result<(), Problem>) {
        match result {
            Ok(()) => {
                self.set_keys(persona, Keys::Ready);
                self.persona = Some(persona);
                self.problem = None;
                self.fresh_login = false;
                self.route();
            }
            Err(p) => self.problem = Some(p),
        }
    }

    pub fn prepare_later(&mut self) {
        self.prepare_later = true;
        self.problem = None;
        self.route();
    }

    pub fn elsewhere_seen(&mut self) {
        self.elsewhere_seen = true;
        self.route();
    }

    fn set_keys(&mut self, persona: Uuid, keys: Keys) {
        if let Some(Standing::SignedIn { personas, .. }) = &mut self.standing
            && let Some(p) = personas.iter_mut().find(|p| p.persona.id == persona)
        {
            p.keys = keys;
        }
    }

    pub fn persona_updated(&mut self, persona: Persona) {
        if let Some(Standing::SignedIn { personas, .. }) = &mut self.standing
            && let Some(p) = personas.iter_mut().find(|p| p.persona.id == persona.id)
        {
            p.persona = persona;
        }
    }

    pub fn persona_deleted(&mut self, id: Uuid) {
        if let Some(Standing::SignedIn { personas, .. }) = &mut self.standing {
            personas.retain(|p| p.persona.id != id);
        }
        self.pick_persona();
    }

    pub fn choose_persona(&mut self, id: Uuid) {
        if self.personas().iter().any(|p| p.persona.id == id) {
            self.persona = Some(id);
        }
    }

    /// Background upkeep's result for server `id`.
    pub fn upkept(&mut self, id: Uuid, result: Result<Option<String>, Problem>) {
        if self.current != Some(id) {
            return;
        }
        match result {
            Ok(notice) => {
                if self.signed_in() {
                    self.status = Status::Connected;
                }
                if notice.is_some() {
                    self.notice = notice;
                }
            }
            Err(Problem::Unreachable(_)) => self.status = Status::Offline,
            Err(Problem::SignedOut) => {
                self.standing = Some(Standing::SignedOut);
                self.status = Status::SignedOut;
            }
            // Logged by the backend for advanced mode; nothing to say.
            Err(_) => {}
        }
    }

    /// The rules for server `id`, fetched by the upkeep loop.
    pub fn rules_loaded(&mut self, id: Uuid, result: Result<Option<ServerRules>, Problem>) {
        if self.current == Some(id)
            && let Ok(rules) = result
        {
            self.rules = Some(rules);
        }
    }

    /// Whether the login must accept the rules before writing again.
    pub fn rules_to_accept(&self) -> bool {
        matches!(&self.rules, Some(Some(r)) if r.must_accept)
    }

    pub fn rules_accepted(&mut self) {
        if let Some(Some(r)) = &mut self.rules {
            r.must_accept = false;
        }
    }

    /// Restarts the upkeep loop, which tries the server at once.
    pub fn try_now(&mut self) {
        self.upkeep_round = self.upkeep_round.wrapping_add(1);
    }

    pub fn go(&mut self, section: Section) {
        self.section = section;
        self.screen = Screen::Frame;
        self.problem = None;
    }

    /// The status line's "Sign in".
    pub fn sign_in_again(&mut self) {
        self.problem = None;
        self.screen = Screen::Account;
    }

    pub fn signed_out(&mut self) {
        // Signing in again lands on Home.
        self.section = Section::Home;
        self.standing = Some(Standing::SignedOut);
        self.status = Status::SignedOut;
        self.persona = None;
        self.screen = Screen::Account;
    }

    pub fn begin_add(&mut self) {
        self.adding = Some(self.current);
        self.problem = None;
        self.screen = Screen::Server;
    }

    /// Stops adding a server; returns the server to reopen, if any.
    pub fn cancel_add(&mut self) -> Option<Uuid> {
        let back = self.adding.take().flatten();
        self.problem = None;
        self.screen = if back.is_some() {
            Screen::Frame
        } else {
            Screen::Server
        };
        back
    }

    /// A server was added and opened (its store is new: nobody's signed in).
    pub fn added(&mut self, backend: &dyn Backend, server: &ServerEntry) {
        self.refresh_servers(backend);
        self.section = Section::Home;
        self.opening(server.id);
        self.opened(server.id, Ok(Standing::SignedOut));
    }

    /// A server was removed. Returns the server to open next, if any.
    pub fn removed(&mut self, backend: &dyn Backend, id: Uuid) -> Option<Uuid> {
        self.refresh_servers(backend);
        if self.current != Some(id) {
            return None;
        }
        self.current = None;
        self.standing = None;
        self.persona = None;
        let next = self.servers.first().map(|s| s.id);
        if next.is_none() {
            self.screen = Screen::Server;
        }
        next
    }

    pub fn settings_changed(&mut self, settings: AppSettings) {
        self.settings = settings;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{Fake, persona};

    fn opened(fake: &Fake) -> Model {
        let mut m = Model::load(fake);
        let id = m.current.unwrap();
        m.opening(id);
        m.opened(id, Ok(fake.standing.borrow().clone()));
        m
    }

    #[test]
    fn a_fresh_install_asks_for_a_server() {
        let m = Model::load(&Fake::empty());
        assert_eq!(m.screen, Screen::Server);
    }

    #[test]
    fn a_ready_persona_opens_the_frame_and_is_in_use() {
        let fake = Fake::signed_in(vec![
            persona("ink_fox", Keys::Elsewhere),
            persona("ink_owl", Keys::Ready),
        ]);
        let m = opened(&fake);
        assert_eq!(m.screen, Screen::Frame);
        assert_eq!(m.status, Status::Connected);
        assert_eq!(m.persona_in_use().unwrap().persona.handle, "ink_owl");
    }

    #[test]
    fn the_first_persona_is_pushed_once_and_skippable() {
        let fake = Fake::signed_in(Vec::new());
        // Opening an old login with no persona: the frame, with its nudge.
        assert_eq!(opened(&fake).screen, Screen::Frame);
        // Just signed in: pushed.
        let mut m = Model::load(&fake);
        let id = m.current.unwrap();
        m.signed_in_now(&fake, id, fake.standing.borrow().clone());
        assert_eq!(m.screen, Screen::FirstPersona);
        m.skip_persona();
        assert_eq!(m.screen, Screen::Frame);
    }

    #[test]
    fn a_new_persona_is_got_ready_then_used() {
        let fake = Fake::signed_in(Vec::new());
        let mut m = opened(&fake);
        let fox = persona("ink_fox", Keys::ToSetUp).persona;
        m.persona_created(fox.clone());
        assert_eq!(m.screen, Screen::Preparing { persona: fox.id });
        m.prepared(fox.id, Err(Problem::Unreachable("w".into())));
        assert_eq!(m.screen, Screen::Preparing { persona: fox.id });
        assert!(m.problem.is_some());
        m.prepared(fox.id, Ok(()));
        assert_eq!(m.screen, Screen::Frame);
        assert_eq!(m.persona, Some(fox.id));
    }

    #[test]
    fn an_interrupted_setup_resumes_unless_put_off() {
        let fake = Fake::signed_in(vec![persona("ink_fox", Keys::ToSetUp)]);
        let mut m = opened(&fake);
        assert!(matches!(m.screen, Screen::Preparing { .. }));
        m.prepare_later();
        assert_eq!(m.screen, Screen::Frame);
    }

    #[test]
    fn keys_on_another_device_are_explained_once() {
        let fake = Fake::signed_in(vec![persona("ink_fox", Keys::Elsewhere)]);
        let mut m = opened(&fake);
        assert_eq!(m.screen, Screen::Elsewhere);
        assert_eq!(m.persona, None);
        m.elsewhere_seen();
        assert_eq!(m.screen, Screen::Frame);
    }

    #[test]
    fn offline_keeps_the_frame_and_stale_results_are_dropped() {
        let fake = Fake::signed_in(vec![persona("ink_fox", Keys::Ready)]);
        let mut m = opened(&fake);
        let id = m.current.unwrap();
        m.upkept(id, Err(Problem::Unreachable("w".into())));
        assert_eq!((m.screen, m.status), (Screen::Frame, Status::Offline));
        // A result for a server no longer in view changes nothing.
        let before = m.clone();
        m.opened(Uuid::new_v4(), Ok(Standing::SignedOut));
        m.upkept(Uuid::new_v4(), Err(Problem::SignedOut));
        assert_eq!(m, before);
        // Signed out while in the frame: the status line says so.
        m.upkept(id, Err(Problem::SignedOut));
        assert_eq!((m.screen, m.status), (Screen::Frame, Status::SignedOut));
    }

    #[test]
    fn unlocking_while_offline_leaves_the_standing_to_load() {
        let fake = Fake::signed_in(vec![persona("ink_fox", Keys::Ready)]);
        let mut m = Model::load(&fake);
        let id = m.current.unwrap();
        m.opening(id);
        m.opened(id, Ok(Standing::Locked));
        assert_eq!(m.screen, Screen::Unlock);
        // Unlocked here, but the server didn't answer.
        m.opened(id, Err(Problem::Unreachable("w".into())));
        assert_eq!((m.screen, m.status), (Screen::Frame, Status::Offline));
        assert_eq!(m.standing, None, "the loop must load it again");
        m.try_now();
        assert_eq!(m.status, Status::Offline);
    }

    #[test]
    fn switching_servers_keeps_the_frame_up() {
        let fake = Fake::signed_in(vec![persona("ink_fox", Keys::Ready)]);
        let other = ServerEntry {
            id: Uuid::new_v4(),
            address: "https://other.example".into(),
            name: "other.example".into(),
            login: Some("ink".into()),
        };
        fake.servers.borrow_mut().push(other.clone());
        let mut m = opened(&fake);
        m.opening(other.id);
        assert_eq!((m.screen, m.status), (Screen::Frame, Status::Connecting));
        assert_eq!(m.persona, None);
        m.opened(other.id, Ok(Standing::Locked));
        assert_eq!(m.screen, Screen::Unlock);
    }

    #[test]
    fn cancelling_an_added_server_goes_back() {
        let fake = Fake::signed_in(vec![persona("ink_fox", Keys::Ready)]);
        let mut m = opened(&fake);
        let first = m.current;
        m.begin_add();
        assert_eq!(m.screen, Screen::Server);
        assert_eq!(m.cancel_add(), first);
        assert_eq!(m.screen, Screen::Frame);
    }

    #[test]
    fn rules_to_accept_follow_the_server_in_view() {
        let fake = Fake::signed_in(vec![persona("ink_fox", Keys::Ready)]);
        let mut m = opened(&fake);
        let id = m.current.unwrap();
        let rules = ServerRules {
            markdown: "Be kind.".into(),
            version: 3,
            must_accept: true,
        };
        m.rules_loaded(Uuid::new_v4(), Ok(Some(rules.clone())));
        assert!(!m.rules_to_accept());
        m.rules_loaded(id, Ok(Some(rules)));
        assert!(m.rules_to_accept());
        m.rules_accepted();
        assert!(!m.rules_to_accept());
        m.opening(Uuid::new_v4());
        assert_eq!(m.rules, None);
    }
}
