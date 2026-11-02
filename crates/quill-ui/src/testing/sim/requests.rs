//! The simulated server's collaboration requests (3e), following
//! `quill-server`'s `requests.rs` and `quill-core`'s `requests` module: a
//! request is filed by the prompt's state (open: pending, busy: held,
//! closed: refused and never shown), the requester sees only waiting,
//! session or no response, and one request waits per login per prompt.
//! This computer pins other writers' identity keys as core does, so a key
//! changed since stops sending and reading until it's accepted.

use std::collections::HashMap;

use quill_proto::{board::PromptState, requests::RequestStatus};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::{Sim, SimServer, Store, refused};
use crate::backend::{
    Inbox, IncomingRequest, KeyChange, PITCH_MAX, Problem, REQUEST_DAYS_MAX, SentRequest, Unread,
    WaitingPrompt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Pending,
    Held,
    Refused,
    Declined,
    Accepted,
    Expired,
}

impl State {
    fn waiting(self) -> bool {
        matches!(
            self,
            State::Pending | State::Held | State::Refused | State::Declined
        )
    }
}

#[derive(Debug, Clone)]
pub(super) struct SimRequest {
    pub(super) id: Uuid,
    pub(super) prompt: Uuid,
    login: String,
    handle: String,
    pitch: String,
    pub(super) state: State,
    received_at: OffsetDateTime,
    expires_at: OffsetDateTime,
    /// Sent before this computer was linked: it holds no key for it.
    before_link: bool,
}

/// One server's requests, and each persona's identity key version.
#[derive(Debug, Clone, Default)]
pub(super) struct Requests {
    pub(super) all: Vec<SimRequest>,
    /// Bumped when a writer starts over with a new identity key.
    key_versions: HashMap<String, u32>,
    seq: i64,
}

impl Requests {
    fn version(&self, handle: &str) -> u32 {
        self.key_versions.get(handle).copied().unwrap_or(1)
    }

    /// Arrival times one second apart, so "newest first" is stable.
    fn next_time(&mut self) -> OffsetDateTime {
        self.seq += 1;
        OffsetDateTime::UNIX_EPOCH + Duration::days(20_000) + Duration::seconds(self.seq)
    }
}

/// This computer's side: pinned identity keys and requests read here.
#[derive(Debug, Clone, Default)]
pub(super) struct Here {
    pins: HashMap<String, u32>,
    read: Vec<Uuid>,
    pub(super) answers_seen: Vec<Uuid>,
}

impl Here {
    /// Core's `check_pin`: pins a key seen for the first time.
    fn check(&mut self, server: &SimServer, handle: &str) -> Result<(), Problem> {
        let now = server.requests.version(handle);
        match self.pins.get(handle) {
            Some(v) if *v != now => Err(Problem::KeyChanged(handle.to_owned())),
            Some(_) => Ok(()),
            None => {
                self.pins.insert(handle.to_owned(), now);
                Ok(())
            }
        }
    }
}

/// The words both writers compare: stand-ins for the identity number, from
/// both personas' handles and key versions, the same whichever side asks.
fn words(a: (&str, u32), b: (&str, u32)) -> Vec<String> {
    const LIST: [&str; 16] = [
        "amber", "birch", "cinder", "delta", "ember", "fjord", "garnet", "harbor", "iris",
        "juniper", "kestrel", "lantern", "meadow", "nectar", "orchid", "pebble",
    ];
    let (first, second) = if a.0 <= b.0 { (a, b) } else { (b, a) };
    let seed = format!("{}:{}|{}:{}", first.0, first.1, second.0, second.1);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    (0..12)
        .map(|i| {
            for b in seed.bytes().chain([i]) {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
            LIST[usize::try_from(h % 16).unwrap_or(0)].to_owned()
        })
        .collect()
}

fn fingerprint(handle: &str, version: u32) -> String {
    format!("{handle} key {version}")
}

/// Ends waiting requests whose time is up, as the server does on each read.
fn expire(server: &mut SimServer, now: OffsetDateTime) {
    for r in &mut server.requests.all {
        if r.state.waiting() && r.expires_at <= now {
            r.state = State::Expired;
        }
    }
}

impl SimServer {
    fn pair_words(&self, a: &str, b: &str) -> Vec<String> {
        words((a, self.requests.version(a)), (b, self.requests.version(b)))
    }

    fn handle_of(&self, persona: Uuid) -> Option<String> {
        self.personas
            .iter()
            .find(|(_, p, _)| p.id == persona)
            .map(|(_, p, _)| p.handle.clone())
    }

    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH + Duration::days(20_000) + Duration::seconds(self.requests.seq)
    }

    /// Files a request, as `POST /requests` does.
    fn file(
        &mut self,
        login: &str,
        handle: &str,
        prompt: Uuid,
        pitch: &str,
        days: u32,
        before_link: bool,
    ) -> Result<SimRequest, Problem> {
        let (state, owner) = match self.prompt_info(prompt) {
            Some((_, owner, state, _)) => (state, owner),
            None => return Err(Problem::Gone),
        };
        if owner == login {
            return Err(refused("That's your own prompt: you can't ask to join it."));
        }
        if !(1..=REQUEST_DAYS_MAX).contains(&days) {
            return Err(refused(&format!(
                "expiresInDays must be 1 to {REQUEST_DAYS_MAX}"
            )));
        }
        let now = self.now();
        expire(self, now);
        if self
            .requests
            .all
            .iter()
            .any(|r| r.login == login && r.prompt == prompt && r.state.waiting())
        {
            return Err(refused(
                "You already have a request waiting on this prompt. Withdraw it to send another.",
            ));
        }
        let received_at = self.requests.next_time();
        let r = SimRequest {
            id: Uuid::new_v4(),
            prompt,
            login: login.to_owned(),
            handle: handle.to_owned(),
            pitch: pitch.to_owned(),
            state: match state {
                PromptState::Open => State::Pending,
                PromptState::Busy => State::Held,
                _ => State::Refused,
            },
            received_at,
            expires_at: received_at + Duration::days(i64::from(days)),
            before_link,
        };
        self.requests.all.push(r.clone());
        Ok(r)
    }
}

// ---- What tests do ----------------------------------------------------------------

impl Sim {
    /// Another writer (made if new) asks to write on `prompt` with `pitch`.
    /// The author's live notice fires if it's pending. Returns its id.
    pub fn request_from(&self, domain: &str, handle: &str, prompt: Uuid, pitch: &str) -> Uuid {
        self.request_as(domain, handle, prompt, pitch, false)
    }

    /// As `request_from`, sent before this computer was linked: it can't
    /// read the pitch.
    pub fn request_before_link(
        &self,
        domain: &str,
        handle: &str,
        prompt: Uuid,
        pitch: &str,
    ) -> Uuid {
        self.request_as(domain, handle, prompt, pitch, true)
    }

    fn request_as(
        &self,
        domain: &str,
        handle: &str,
        prompt: Uuid,
        pitch: &str,
        before_link: bool,
    ) -> Uuid {
        self.take_handle_if_new(domain, handle);
        let r = self.server(domain, |s| {
            let login = s
                .personas
                .iter()
                .find(|(_, p, _)| p.handle == handle)
                .map(|(l, _, _)| l.clone())
                .expect("the persona");
            s.file(&login, handle, prompt, pitch, REQUEST_DAYS_MAX, before_link)
                .unwrap_or_else(|p| panic!("requesting: {p}"))
        });
        if r.state == State::Pending {
            self.live();
        }
        r.id
    }

    fn take_handle_if_new(&self, domain: &str, handle: &str) {
        let known = self.server(domain, |s| {
            s.personas.iter().any(|(_, p, _)| p.handle == handle)
        });
        if !known {
            self.take_handle(domain, handle);
        }
    }

    /// The prompt's author answers one of this login's requests, from
    /// their own devices. Accepting fires the live notice.
    pub fn answer_request(&self, domain: &str, id: Uuid, accept: bool) {
        self.server(domain, |s| {
            let r = s
                .requests
                .all
                .iter_mut()
                .find(|r| r.id == id)
                .expect("the request");
            r.state = if accept {
                State::Accepted
            } else {
                State::Declined
            };
        });
        if accept {
            self.live();
        }
    }

    /// Time runs out for every waiting request on a server.
    pub fn expire_requests(&self, domain: &str) {
        self.server(domain, |s| {
            for r in &mut s.requests.all {
                if r.state.waiting() {
                    r.state = State::Expired;
                }
            }
        });
    }

    /// `handle` starts over with a new identity key: this computer's pin
    /// no longer matches.
    pub fn change_key(&self, domain: &str, handle: &str) {
        self.server(domain, |s| {
            let v = s.requests.version(handle);
            s.requests.key_versions.insert(handle.to_owned(), v + 1);
        });
    }

    /// The requester withdraws a request (from their own devices).
    pub fn withdraw_elsewhere(&self, domain: &str, id: Uuid) {
        self.server(domain, |s| s.requests.all.retain(|r| r.id != id));
    }

    /// Each request on a server, oldest first: its id, the requester's
    /// handle, the prompt and its state.
    pub fn requests_on(&self, domain: &str) -> Vec<(Uuid, String, Uuid, State)> {
        self.server(domain, |s| {
            s.requests
                .all
                .iter()
                .map(|r| (r.id, r.handle.clone(), r.prompt, r.state))
                .collect()
        })
    }

    /// The words `a` and `b` compare, for their current keys.
    pub fn key_words(&self, domain: &str, a: &str, b: &str) -> Vec<String> {
        self.server(domain, |s| s.pair_words(a, b))
    }

    // ---- The backend's side ------------------------------------------------------

    pub(super) fn sim_send_request(
        &self,
        persona: Uuid,
        prompt: Uuid,
        pitch: &str,
        days: u32,
    ) -> Result<SentRequest, Problem> {
        let pitch = pitch.trim();
        if pitch.is_empty() {
            return Err(refused("say something in the pitch"));
        }
        if pitch.chars().count() > PITCH_MAX {
            return Err(refused(&format!(
                "the pitch is too long (at most {PITCH_MAX} characters)"
            )));
        }
        let (_, store, _) = self.reach()?;
        if !store.keys.contains(&persona) {
            return Err(Problem::Failed {
                detail: "no keys on this device".into(),
            });
        }
        // Core pins a key seen for the first time even if sending fails.
        let mut here = self.here();
        let sent = self.writing(|server, login| {
            let handle = server.handle_of(persona).ok_or(Problem::Gone)?;
            let Some((title, _, _, author)) = server.prompt_info(prompt) else {
                return Err(Problem::Gone);
            };
            let published = server
                .personas
                .iter()
                .any(|(_, p, published)| p.handle == author && *published);
            // Core checks the author's key before claiming their devices.
            here.check(server, &author)?;
            if !published {
                return Err(refused(&format!(
                    "@{author} has no devices that can take a request right now. Try again later."
                )));
            }
            let r = server.file(login, &handle, prompt, pitch, days, false)?;
            Ok(SentRequest {
                id: r.id,
                prompt: Some((prompt, title)),
                persona: handle,
                status: RequestStatus::Waiting,
                expires_at: r.expires_at,
            })
        });
        self.set_here(here);
        sent
    }

    pub(super) fn sim_sent_requests(&self) -> Result<Vec<SentRequest>, Problem> {
        self.writing(|server, login| {
            let now = server.now();
            expire(server, now);
            let mut mine: Vec<&SimRequest> = server
                .requests
                .all
                .iter()
                .filter(|r| r.login == login)
                .collect();
            mine.sort_by_key(|r| std::cmp::Reverse(r.received_at));
            Ok(mine
                .into_iter()
                .map(|r| SentRequest {
                    id: r.id,
                    prompt: server
                        .prompt_info(r.prompt)
                        .map(|(title, ..)| (r.prompt, title)),
                    persona: r.handle.clone(),
                    status: match r.state {
                        State::Accepted => RequestStatus::Session,
                        State::Expired => RequestStatus::NoResponse,
                        _ => RequestStatus::Waiting,
                    },
                    expires_at: r.expires_at,
                })
                .collect())
        })
    }

    pub(super) fn sim_withdraw(&self, id: Uuid) -> Result<(), Problem> {
        self.writing(|server, login| {
            let r = server
                .requests
                .all
                .iter()
                .find(|r| r.id == id && r.login == login)
                .ok_or(Problem::Gone)?;
            if r.state == State::Accepted {
                return Err(refused("It's a session now; leave the session instead."));
            }
            server.requests.all.retain(|r| r.id != id);
            Ok(())
        })
    }

    pub(super) fn sim_waiting(&self) -> Result<Vec<WaitingPrompt>, Problem> {
        self.writing(|server, login| {
            let now = server.now();
            expire(server, now);
            let mut out: Vec<WaitingPrompt> = Vec::new();
            for r in &server.requests.all {
                let Some((title, owner, ..)) = server.prompt_info(r.prompt) else {
                    continue;
                };
                if owner != login || !matches!(r.state, State::Pending | State::Held) {
                    continue;
                }
                let w = match out.iter_mut().find(|w| w.prompt == r.prompt) {
                    Some(w) => w,
                    None => {
                        out.push(WaitingPrompt {
                            prompt: r.prompt,
                            title,
                            pending: 0,
                            held: 0,
                            newest: r.received_at,
                        });
                        out.last_mut().expect("just pushed")
                    }
                };
                if r.state == State::Pending {
                    w.pending += 1;
                } else {
                    w.held += 1;
                }
                w.newest = w.newest.max(r.received_at);
            }
            out.sort_by_key(|w| std::cmp::Reverse(w.newest));
            Ok(out)
        })
    }

    pub(super) fn sim_inbox(&self, prompt: Uuid, held: bool) -> Result<Inbox, Problem> {
        let mut here = self.here();
        let inbox = self.writing(|server, login| {
            match server.prompt_info(prompt) {
                Some((_, owner, ..)) if owner == login => {}
                _ => return Err(Problem::Gone),
            }
            let now = server.now();
            expire(server, now);
            if held {
                for r in &mut server.requests.all {
                    if r.prompt == prompt && r.state == State::Held {
                        r.state = State::Pending;
                    }
                }
            }
            let mut requests = Vec::new();
            for r in server
                .requests
                .all
                .iter()
                .filter(|r| r.prompt == prompt && r.state == State::Pending)
            {
                let pitch = if here.read.contains(&r.id) {
                    Ok(r.pitch.clone())
                } else if r.before_link {
                    Err(Unread::NotForThisComputer)
                } else if here.check(server, &r.handle).is_err() {
                    Err(Unread::KeyChanged)
                } else {
                    here.read.push(r.id);
                    Ok(r.pitch.clone())
                };
                requests.push(IncomingRequest {
                    id: r.id,
                    handle: r.handle.clone(),
                    display_name: None,
                    received_at: r.received_at,
                    expires_at: r.expires_at,
                    pitch,
                });
            }
            let held = server
                .requests
                .all
                .iter()
                .filter(|r| r.prompt == prompt && r.state == State::Held)
                .count();
            Ok(Inbox {
                requests,
                held: u32::try_from(held).unwrap_or(u32::MAX),
            })
        })?;
        self.set_here(here);
        Ok(inbox)
    }

    /// Accepting or declining, as the author: `accept` needs the pitch
    /// read here first, as core's does.
    pub(super) fn sim_answer(&self, id: Uuid, accept: bool) -> Result<(), Problem> {
        let read_here = self.here().read.contains(&id);
        self.writing(|server, login| {
            let r = server
                .requests
                .all
                .iter()
                .find(|r| r.id == id)
                .ok_or(Problem::Gone)?;
            let owner = server.prompt_info(r.prompt).map(|(_, o, ..)| o);
            if owner.as_deref() != Some(login) || r.state == State::Refused {
                return Err(Problem::Gone);
            }
            if accept && !read_here {
                return Err(Problem::Failed {
                    detail: "read the request on this device first".into(),
                });
            }
            let state = match (r.state, accept) {
                (State::Pending | State::Held, true) => State::Accepted,
                (State::Pending | State::Held, false) => State::Declined,
                // Accepted already, perhaps on another device: core's
                // accept takes that as done.
                (State::Accepted, true) => State::Accepted,
                (State::Accepted, false) => return Err(refused("It's already a session.")),
                (State::Declined, _) => return Err(refused("It was declined.")),
                _ => return Err(refused("It expired.")),
            };
            if let Some(r) = server.requests.all.iter_mut().find(|r| r.id == id) {
                r.state = state;
            }
            Ok(())
        })
    }

    pub(super) fn sim_key_change(
        &self,
        handle: &str,
        as_persona: Uuid,
    ) -> Result<Option<KeyChange>, Problem> {
        let (_, _, server) = self.reach()?;
        let me = server.handle_of(as_persona).ok_or(Problem::Gone)?;
        let now = server.requests.version(handle);
        Ok(match self.here().pins.get(handle) {
            Some(v) if *v != now => Some(KeyChange {
                handle: handle.to_owned(),
                words: server.pair_words(&me, handle),
                fingerprint: fingerprint(handle, now),
            }),
            _ => None,
        })
    }

    pub(super) fn sim_identity_words(
        &self,
        handle: &str,
        as_persona: Uuid,
    ) -> Result<Option<Vec<String>>, Problem> {
        let (_, _, server) = self.reach()?;
        let me = server.handle_of(as_persona).ok_or(Problem::Gone)?;
        let handle = handle.trim().trim_start_matches('@');
        match server.personas.iter().find(|(_, p, _)| p.handle == handle) {
            None => Err(Problem::Gone),
            Some((_, _, false)) => Ok(None),
            Some(_) => Ok(Some(server.pair_words(&me, handle))),
        }
    }

    pub(super) fn sim_accept_key_change(&self, handle: &str, checked: &str) -> Result<(), Problem> {
        let (_, _, server) = self.reach()?;
        let now = server.requests.version(handle);
        if fingerprint(handle, now) != checked {
            return Err(Problem::KeyChanged(handle.to_owned()));
        }
        let mut here = self.here();
        here.pins.insert(handle.to_owned(), now);
        self.set_here(here);
        Ok(())
    }

    fn here(&self) -> Here {
        self.open_store()
            .map(|(_, store, _)| store.requests)
            .unwrap_or_default()
    }

    fn set_here(&self, here: Here) {
        self.with_server_mut(|_, store: &mut Store| store.requests = here);
    }
}
