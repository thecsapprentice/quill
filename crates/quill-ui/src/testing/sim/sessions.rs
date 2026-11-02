//! The simulated server's sessions (3f), following `quill-server`'s
//! sessions and `quill-core`'s `sessions` and `history`. An accepted
//! request becomes a session between its two writers, starting with the
//! pitch in the planning channel. Each note has a number (its place in the
//! log); edits keep every version; a deletion keeps them too. This
//! computer keeps how far it has shown each channel, and its standing.

use std::collections::HashMap;

use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::{Sim, SimServer, refused};
use crate::backend::{
    ChannelView, Note, Problem, SessionStanding, SessionSummary, SessionView, Writer, codes,
};

/// The channels every session has.
const DEFAULT_CHANNELS: [&str; 2] = ["story", "planning"];

#[derive(Debug, Clone)]
pub(super) struct SimNote {
    seq: u64,
    /// The author's handle.
    author: String,
    channel: String,
    versions: Vec<String>,
    deleted: bool,
    reply_to: Option<u64>,
    at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub(super) struct SimSession {
    pub(super) id: String,
    prompt: Uuid,
    /// The two writers still in it: login and handle.
    members: Vec<(String, String)>,
    /// Both writers, even after one leaves: handle, for naming notes.
    writers: Vec<String>,
    notes: Vec<SimNote>,
    /// Named channels beyond the defaults: name and description.
    channels: Vec<(String, Option<String>)>,
    ended: bool,
    seq: u64,
}

/// One server's sessions.
#[derive(Debug, Clone, Default)]
pub(super) struct Sessions {
    pub(super) all: Vec<SimSession>,
    count: u32,
}

/// This computer's side: how far each channel is shown, and standings
/// other than joined.
#[derive(Debug, Clone, Default)]
pub(super) struct Here {
    shown: HashMap<(String, String), u64>,
    standing: HashMap<String, SessionStanding>,
}

fn base_time() -> OffsetDateTime {
    OffsetDateTime::UNIX_EPOCH + Duration::days(20_100)
}

impl SimServer {
    /// An accepted request becomes a session, starting with its pitch.
    pub(super) fn begin_session(
        &mut self,
        prompt: Uuid,
        requester: (String, String),
        author: (String, String),
        pitch: &str,
    ) -> String {
        self.sessions.count += 1;
        let id = format!("c2Vzc2lvbi{:04}", self.sessions.count);
        let mut s = SimSession {
            id: id.clone(),
            prompt,
            writers: vec![requester.1.clone(), author.1.clone()],
            members: vec![requester.clone(), author],
            notes: Vec::new(),
            channels: Vec::new(),
            ended: false,
            seq: 0,
        };
        s.write(&requester.1, "planning", pitch, None);
        self.sessions.all.push(s);
        id
    }

    fn session_mut(&mut self, id: &str) -> &mut SimSession {
        self.sessions
            .all
            .iter_mut()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no session {id}"))
    }
}

impl SimSession {
    fn write(&mut self, author: &str, channel: &str, text: &str, reply_to: Option<u64>) -> u64 {
        self.seq += 1;
        if !DEFAULT_CHANNELS.contains(&channel) && !self.channels.iter().any(|(n, _)| n == channel)
        {
            self.channels.push((channel.to_owned(), None));
        }
        self.notes.push(SimNote {
            seq: self.seq,
            author: author.to_owned(),
            channel: channel.to_owned(),
            versions: vec![text.to_owned()],
            deleted: false,
            reply_to,
            at: base_time() + Duration::minutes(i64::try_from(self.seq).unwrap_or(0)),
        });
        self.seq
    }

    fn note_mut(&mut self, seq: u64) -> &mut SimNote {
        self.notes
            .iter_mut()
            .find(|n| n.seq == seq)
            .unwrap_or_else(|| panic!("no note #{seq}"))
    }

    fn mine<'a>(&'a self, login: &str) -> Option<&'a (String, String)> {
        self.members.iter().find(|(l, _)| l == login)
    }
}

// ---- What tests do --------------------------------------------------------------------

impl Sim {
    /// A session between the open login's persona `mine` and `partner`
    /// (made if new), on a prompt of the partner's titled `title`.
    /// Returns its id.
    pub fn session_with(&self, domain: &str, mine: &str, partner: &str, title: &str) -> String {
        let prompt = self.post_as(
            domain,
            partner,
            title,
            "A prompt to write together.",
            &[],
            quill_proto::board::Maturity::General,
        );
        let login = self.login_of(domain, mine);
        let partner_login = self.login_of(domain, partner);
        let id = self.server(domain, |s| {
            s.begin_session(
                prompt,
                (login, mine.to_owned()),
                (partner_login, partner.to_owned()),
                "Shall we?",
            )
        });
        self.live();
        id
    }

    fn login_of(&self, domain: &str, handle: &str) -> String {
        self.server(domain, |s| {
            s.personas
                .iter()
                .find(|(_, p, _)| p.handle == handle)
                .map(|(l, _, _)| l.clone())
                .unwrap_or_else(|| panic!("no persona @{handle}"))
        })
    }

    /// `handle` writes a note in a session; the live notice fires. Returns
    /// its number.
    pub fn write_as(
        &self,
        domain: &str,
        session: &str,
        handle: &str,
        channel: &str,
        text: &str,
    ) -> u64 {
        let seq = self.server(domain, |s| {
            s.session_mut(session).write(handle, channel, text, None)
        });
        self.live();
        seq
    }

    /// As `write_as`, answering note `to`.
    pub fn reply_as(
        &self,
        domain: &str,
        session: &str,
        handle: &str,
        channel: &str,
        text: &str,
        to: u64,
    ) -> u64 {
        let seq = self.server(domain, |s| {
            s.session_mut(session)
                .write(handle, channel, text, Some(to))
        });
        self.live();
        seq
    }

    /// A note is changed by its author; every version is kept.
    pub fn edit_note(&self, domain: &str, session: &str, seq: u64, text: &str) {
        self.server(domain, |s| {
            s.session_mut(session)
                .note_mut(seq)
                .versions
                .push(text.into())
        });
        self.live();
    }

    /// A note is deleted by its author; it shows as deleted.
    pub fn delete_note(&self, domain: &str, session: &str, seq: u64) {
        self.server(domain, |s| {
            s.session_mut(session).note_mut(seq).deleted = true
        });
        self.live();
    }

    /// A channel is named, or described again.
    pub fn name_channel(&self, domain: &str, session: &str, name: &str, summary: &str) {
        self.server(domain, |s| {
            let session = s.session_mut(session);
            match session.channels.iter_mut().find(|(n, _)| n == name) {
                Some(c) => c.1 = Some(summary.to_owned()),
                None => session
                    .channels
                    .push((name.to_owned(), Some(summary.to_owned()))),
            }
        });
        self.live();
    }

    /// `handle` leaves: with two writers, that ends the session for both.
    pub fn leave_as(&self, domain: &str, session: &str, handle: &str) {
        self.server(domain, |s| {
            let session = s.session_mut(session);
            session.members.retain(|(_, h)| h != handle);
            session.ended = true;
        });
        self.live();
    }

    /// This computer's standing in a session becomes `standing`.
    pub fn set_session_standing(&self, session: &str, standing: SessionStanding) {
        self.with_server_mut(|_, store| {
            store.sessions.standing.insert(session.to_owned(), standing);
        });
    }

    /// The ids of the sessions a login takes part in.
    pub fn sessions_of(&self, domain: &str, login: &str) -> Vec<String> {
        self.server(domain, |s| {
            s.sessions
                .all
                .iter()
                .filter(|x| x.members.iter().any(|(l, _)| l == login))
                .map(|x| x.id.clone())
                .collect()
        })
    }

    // ---- The backend's side ---------------------------------------------------------

    fn writer_named(server: &SimServer, handle: &str) -> Writer {
        Writer {
            handle: handle.to_owned(),
            display_name: server
                .personas
                .iter()
                .find(|(_, p, _)| p.handle == handle)
                .and_then(|(_, p, _)| p.display_name.clone()),
        }
    }

    fn summary_of(&self, server: &SimServer, s: &SimSession, login: &str) -> SessionSummary {
        let here = self.sessions_here();
        let (_, me) = s.mine(login).cloned().unwrap_or_default();
        let partner = s
            .members
            .iter()
            .find(|(l, _)| l != login)
            .map(|(_, h)| h.clone());
        let channels = Self::channels_of(&here, s, &me);
        let (_, store, _) = self.open_store().expect("a store");
        let mine = server
            .personas
            .iter()
            .find(|(_, p, _)| p.handle == me)
            .map(|(_, p, _)| p.id);
        let standing = here.standing.get(&s.id).cloned().unwrap_or_else(|| {
            if mine.is_some_and(|id| store.keys.contains(&id)) {
                SessionStanding::Joined
            } else {
                SessionStanding::NoKeys
            }
        });
        SessionSummary {
            id: s.id.clone(),
            me: Self::writer_named(server, &me),
            partner: partner.map(|h| Self::writer_named(server, &h)),
            prompt: server
                .prompt_info(s.prompt)
                .map(|(title, ..)| (s.prompt, title)),
            standing,
            ended: s.ended,
            notes: s.notes.len(),
            unseen: channels.iter().map(|c| c.unseen).sum(),
            latest: s.notes.last().map(|n| n.at),
        }
    }

    fn channels_of(here: &Here, s: &SimSession, me: &str) -> Vec<ChannelView> {
        let names = DEFAULT_CHANNELS
            .iter()
            .map(|n| ((*n).to_owned(), None))
            .chain(s.channels.iter().cloned());
        names
            .map(|(name, summary)| {
                let shown = here
                    .shown
                    .get(&(s.id.clone(), name.clone()))
                    .copied()
                    .unwrap_or(0);
                let mine = s.notes.iter().filter(|n| n.channel == name);
                ChannelView {
                    notes: mine.clone().count(),
                    unseen: mine.filter(|n| n.author != me && n.seq > shown).count(),
                    name,
                    summary,
                }
            })
            .collect()
    }

    fn sessions_here(&self) -> Here {
        self.open_store()
            .map(|(_, store, _)| store.sessions)
            .unwrap_or_default()
    }

    pub(super) fn sim_sessions(&self) -> Result<Vec<SessionSummary>, Problem> {
        let (login, server) = self.signed_in()?;
        let mut out: Vec<SessionSummary> = server
            .sessions
            .all
            .iter()
            .filter(|s| {
                s.mine(&login).is_some()
                    || s.ended
                        && s.writers.iter().any(|w| {
                            server
                                .personas
                                .iter()
                                .any(|(l, p, _)| l == &login && &p.handle == w)
                        })
            })
            .map(|s| self.summary_of(&server, s, &login))
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.latest));
        Ok(out)
    }

    pub(super) fn sim_session(&self, id: &str) -> Result<SessionView, Problem> {
        let (login, server) = self.signed_in()?;
        let s = server
            .sessions
            .all
            .iter()
            .find(|s| s.id == id)
            .ok_or(Problem::Gone)?;
        let summary = self.summary_of(&server, s, &login);
        let refuse = |text: &str| Err(refused(codes::SESSION_CHECK_FAILED, text));
        match &summary.standing {
            SessionStanding::Joined => {}
            SessionStanding::Waiting => {
                return refuse(
                    "this device isn't in the session yet: another of your devices for this persona adds it when it next runs",
                );
            }
            SessionStanding::NoKeys => return refuse("this persona has no keys on this device"),
            SessionStanding::Rejoining => {
                return refuse(
                    "this device was away too long to follow the session: it's waiting to be let back in by another of your devices or the other writer",
                );
            }
            SessionStanding::Broken { code, detail } => {
                return Err(refused(*code, detail));
            }
        }
        let me = summary.me.handle.clone();
        let notes = s
            .notes
            .iter()
            .map(|n| Note {
                seq: n.seq,
                mine: n.author == me,
                author: Self::writer_named(&server, &n.author),
                channel: n.channel.clone(),
                content: n.versions.last().cloned().unwrap_or_default(),
                deleted: n.deleted,
                edited: n.versions.len() > 1,
                reply_to: n.reply_to,
                at: n.at,
                attachments: 0,
            })
            .collect();
        Ok(SessionView {
            channels: Self::channels_of(&self.sessions_here(), s, &me),
            summary,
            notes,
            held_back: false,
            history_missing: false,
        })
    }

    pub(super) fn sim_mark_shown(&self, id: &str, channel: &str, upto: u64) -> Result<(), Problem> {
        self.with_server_mut(|_, store| {
            let e = store
                .sessions
                .shown
                .entry((id.to_owned(), channel.to_owned()))
                .or_default();
            *e = (*e).max(upto);
        });
        Ok(())
    }
}
