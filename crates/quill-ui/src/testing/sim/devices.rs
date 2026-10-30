//! The simulated device group (3d): the login's devices, links waiting for
//! approval, and what another of the writer's devices does (approve this
//! computer, wait to be approved, remove this one). It follows
//! `quill-core`'s `devices` module: one group per login, made when a
//! second device joins; names travel only inside it.

use uuid::Uuid;

use super::{Problem, Sim, SimServer, Store, refused};
use crate::backend::{Approval, Device, LinkStart, PersonaKeys, RemoveReason};

/// Words a link can show; each new link takes the next six.
const WORDS: [&str; 24] = [
    "acorn", "amber", "basket", "candle", "copper", "dawn", "ember", "fable", "garden", "harbor",
    "island", "juniper", "kettle", "lantern", "meadow", "nectar", "orchard", "pebble", "quiet",
    "river", "saddle", "thistle", "violet", "willow",
];

/// One of a login's devices, as the group knows it.
#[derive(Debug, Clone)]
pub(super) struct SimDevice {
    pub id: Uuid,
    pub login: String,
    pub name: Option<String>,
    pub since: u64,
    /// Personas it holds keys for.
    pub personas: Vec<Uuid>,
}

/// A link opened by a new device, waiting for one in use.
#[derive(Debug, Clone)]
pub(super) struct SimLink {
    pub id: Uuid,
    pub login: String,
    pub words: Vec<String>,
    /// The words the new device shows once answered.
    pub confirm: Option<Vec<String>>,
    pub expired: bool,
    /// Opened by this computer (rather than another one out there).
    pub from_here: bool,
}

/// The server's side: devices and links for every login.
#[derive(Debug, Clone, Default)]
pub(super) struct Devices {
    pub list: Vec<SimDevice>,
    pub links: Vec<SimLink>,
    /// Reasons given when devices were removed, oldest first.
    pub removed: Vec<RemoveReason>,
    seq: usize,
}

impl Devices {
    fn next_words(&mut self) -> Vec<String> {
        let start = self.seq * 6;
        self.seq += 1;
        (start..start + 6)
            .map(|i| WORDS[i % WORDS.len()].to_owned())
            .collect()
    }

    fn waiting(&self, login: &str) -> Vec<&SimLink> {
        self.links
            .iter()
            .filter(|l| l.login == login && l.confirm.is_none() && !l.expired && !l.from_here)
            .collect()
    }
}

/// This computer's side, kept in its store.
#[derive(Debug, Clone, Default)]
pub(super) struct Here {
    /// Its id among the login's devices, once it's in the group.
    pub device: Option<Uuid>,
    pub name: Option<String>,
    /// The link it opened.
    pub link: Option<Uuid>,
    /// Another device removed it.
    pub removed: bool,
}

/// Seconds for "now": devices made in a test count from here.
const NOW: u64 = 1_790_000_000;

/// Puts this computer in its login's group, if it isn't yet.
fn join_here(server: &mut SimServer, store: &mut Store, login: &str) -> Uuid {
    if let Some(id) = store.here.device {
        return id;
    }
    let id = Uuid::new_v4();
    server.devices.list.push(SimDevice {
        id,
        login: login.to_owned(),
        name: store.here.name.clone(),
        since: NOW,
        personas: store.keys.clone(),
    });
    store.here.device = Some(id);
    id
}

impl Sim {
    /// Another of `login`'s devices, holding keys for `handles`. The login
    /// then has a device group, as linking would have made.
    pub fn other_device(&self, domain: &str, login: &str, name: &str, handles: &[&str]) -> Uuid {
        let id = Uuid::new_v4();
        self.server(domain, |s| {
            let personas = s
                .personas
                .iter()
                .filter(|(_, p, _)| handles.contains(&p.handle.as_str()))
                .map(|(_, p, _)| p.id)
                .collect();
            s.devices.list.push(SimDevice {
                id,
                login: login.into(),
                name: Some(name.into()).filter(|n: &String| !n.is_empty()),
                since: NOW - 86_400 * 30,
                personas,
            });
        });
        id
    }

    /// A new computer out there opens a link for `login`; returns the
    /// words it shows, to type on this computer.
    pub fn link_waiting_elsewhere(&self, domain: &str, login: &str) -> Vec<String> {
        self.server(domain, |s| {
            let words = s.devices.next_words();
            s.devices.links.push(SimLink {
                id: Uuid::new_v4(),
                login: login.into(),
                words: words.clone(),
                confirm: None,
                expired: false,
                from_here: false,
            });
            words
        })
    }

    /// Another device approves this computer's link. Returns the words it
    /// shows, which this computer shows too unless `forged` (a server
    /// handing this computer a group of its own making).
    pub fn approve_from_another_device(&self, domain: &str, forged: bool) -> Vec<String> {
        let mut s = self.0.borrow_mut();
        let st = &mut *s;
        let store = st
            .stores
            .values()
            .find(|v| v.address == super::address(domain))
            .expect("a store for the server");
        let link = store.here.link.expect("this computer opened a link");
        let server = st.servers.get_mut(&super::address(domain)).expect("server");
        let shown = server.devices.next_words();
        let given = if forged {
            server.devices.next_words()
        } else {
            shown.clone()
        };
        if let Some(l) = server.devices.links.iter_mut().find(|l| l.id == link) {
            l.confirm = Some(given);
        }
        shown
    }

    /// Every waiting link on a server expires.
    pub fn expire_links(&self, domain: &str) {
        self.server(domain, |s| {
            for l in &mut s.devices.links {
                l.expired = true;
            }
        });
    }

    /// Another device removes this computer from the login's devices: its
    /// keys go, and upkeep says so.
    pub fn remove_this_computer(&self, domain: &str) {
        let mut s = self.0.borrow_mut();
        let st = &mut *s;
        let store = st
            .stores
            .values_mut()
            .find(|v| v.address == super::address(domain))
            .expect("a store for the server");
        let server = st.servers.get_mut(&super::address(domain)).expect("server");
        if let Some(id) = store.here.device {
            server.devices.list.retain(|d| d.id != id);
        }
        store.keys.clear();
        store.here.removed = true;
    }

    /// The login's devices as the server's group knows them: name and the
    /// handles each can write as.
    pub fn devices_of(&self, domain: &str, login: &str) -> Vec<(Option<String>, Vec<String>)> {
        self.server(domain, |s| {
            s.devices
                .list
                .iter()
                .filter(|d| d.login == login)
                .map(|d| {
                    let handles = d
                        .personas
                        .iter()
                        .filter_map(|id| s.personas.iter().find(|(_, p, _)| p.id == *id))
                        .map(|(_, p, _)| p.handle.clone())
                        .collect();
                    (d.name.clone(), handles)
                })
                .collect()
        })
    }

    /// Why devices were removed on a server, oldest first.
    pub fn removal_reasons(&self, domain: &str) -> Vec<RemoveReason> {
        self.server(domain, |s| s.devices.removed.clone())
    }

    // ---- The Backend side, called from `mod.rs` ------------------------------------------------

    pub(super) fn sim_device_name(&self) -> Option<String> {
        self.open_store()
            .ok()
            .and_then(|(_, store, _)| store.here.name)
    }

    pub(super) fn sim_rename_device(&self, name: &str) -> Result<(), Problem> {
        let name = name.trim().to_owned();
        if name.chars().count() > crate::backend::DEVICE_NAME_MAX {
            return Err(refused("A device name is at most 40 characters."));
        }
        self.reach()?;
        let name = Some(name).filter(|n| !n.is_empty());
        self.with_server_mut(|server, store| {
            store.here.name = name.clone();
            if let Some(id) = store.here.device
                && let Some(d) = server.devices.list.iter_mut().find(|d| d.id == id)
            {
                d.name = name;
            }
        });
        Ok(())
    }

    pub(super) fn sim_devices(&self) -> Result<Vec<Device>, Problem> {
        let (_, store, server) = self.reach()?;
        let login = store.login.clone().ok_or(Problem::SignedOut)?;
        // No group until a second device joins (as in quill-core).
        let Some(here) = store.here.device else {
            return Ok(Vec::new());
        };
        Ok(server
            .devices
            .list
            .iter()
            .filter(|d| d.login == login)
            .map(|d| Device {
                id: d.id,
                this_device: d.id == here,
                name: d.name.clone(),
                since: Some(d.since),
                personas: d
                    .personas
                    .iter()
                    .filter_map(|id| server.personas.iter().find(|(_, p, _)| p.id == *id))
                    .map(|(_, p, _)| p.handle.clone())
                    .collect(),
            })
            .collect())
    }

    pub(super) fn sim_remove_device(&self, id: Uuid, reason: RemoveReason) -> Result<(), Problem> {
        self.writing(|server, _| {
            server.devices.removed.push(reason);
            let before = server.devices.list.len();
            server.devices.list.retain(|d| d.id != id);
            if server.devices.list.len() == before {
                return Err(Problem::Gone);
            }
            Ok(())
        })
    }

    pub(super) fn sim_start_link(&self, name: &str) -> Result<LinkStart, Problem> {
        self.sim_rename_device(name)?;
        let (domain, login) = self.signed_in().map(|(l, s)| (s.domain, l))?;
        self.with_server_mut(|server, store| {
            if store.here.device.is_some() && !store.here.removed {
                return Err(refused("This computer is already one of your devices."));
            }
            // Linking again after a removal starts afresh.
            store.here.device = None;
            store.here.removed = false;
            let words = server.devices.next_words();
            let id = Uuid::new_v4();
            server.devices.links.push(SimLink {
                id,
                login,
                words: words.clone(),
                confirm: None,
                expired: false,
                from_here: true,
            });
            store.here.link = Some(id);
            Ok(LinkStart {
                code: format!("quill-link:1:{domain}:{id}:c2ltdWxhdGVk"),
                words,
                expires_in_secs: 600,
            })
        })
    }

    pub(super) fn sim_link_answer(&self) -> Result<Option<Vec<String>>, Problem> {
        let (_, store, server) = self.reach()?;
        let id = store.here.link.ok_or(Problem::Gone)?;
        let link = server.devices.links.iter().find(|l| l.id == id);
        match link {
            None => Err(Problem::Gone),
            Some(l) if l.expired && l.confirm.is_none() => Err(Problem::Gone),
            Some(l) => Ok(l.confirm.clone()),
        }
    }

    pub(super) fn sim_finish_link(&self) -> Result<(), Problem> {
        self.reach()?;
        self.with_server_mut(|server, store| {
            let login = store.login.clone().unwrap_or_default();
            // Keys for every persona the login's devices hold.
            let granted: Vec<Uuid> = server
                .devices
                .list
                .iter()
                .filter(|d| d.login == login)
                .flat_map(|d| d.personas.clone())
                .collect();
            for id in granted {
                if !store.keys.contains(&id) {
                    store.keys.push(id);
                }
            }
            store.here.link = None;
            join_here(server, store, &login);
            let id = store.here.device;
            let keys = store.keys.clone();
            if let Some(d) = server.devices.list.iter_mut().find(|d| Some(d.id) == id) {
                d.personas = keys;
            }
        });
        Ok(())
    }

    pub(super) fn sim_abandon_link(&self) -> Result<(), Problem> {
        self.with_server_mut(|_, store| store.here.link = None);
        Ok(())
    }

    pub(super) fn sim_approve_link(&self, typed: &str) -> Result<Approval, Problem> {
        let (login, _) = self.signed_in()?;
        let typed = typed.trim().to_lowercase();
        let by_code = typed.starts_with("quill-link:");
        let words: Vec<String> = typed.split_whitespace().map(str::to_owned).collect();
        if !by_code && words.len() != 6 {
            return Err(refused(
                "Type the six words the new computer shows, or paste its whole link code.",
            ));
        }
        self.with_server_mut(|server, store| {
            let link_id = if by_code {
                let found = server
                    .devices
                    .waiting(&login)
                    .into_iter()
                    .find(|l| typed.contains(&l.id.to_string()));
                match found {
                    Some(l) => l.id,
                    None => return Ok(Approval::NoneWaiting),
                }
            } else {
                let waiting = server.devices.waiting(&login);
                match waiting.as_slice() {
                    [] => return Ok(Approval::NoneWaiting),
                    [one] if one.words != words => return Ok(Approval::WordsDiffer),
                    [one] => one.id,
                    many => return Ok(Approval::Several(many.len())),
                }
            };
            join_here(server, store, &login);
            let confirm = server.devices.next_words();
            if let Some(l) = server.devices.links.iter_mut().find(|l| l.id == link_id) {
                l.confirm = Some(confirm.clone());
            }
            // The new device joins, nameless until it says otherwise.
            server.devices.list.push(SimDevice {
                id: Uuid::new_v4(),
                login: login.clone(),
                name: None,
                since: NOW,
                personas: store.keys.clone(),
            });
            Ok(Approval::Approved { confirm })
        })
    }

    pub(super) fn sim_waiting_links(&self) -> Result<usize, Problem> {
        let (login, server) = self.signed_in()?;
        Ok(server.devices.waiting(&login).len())
    }

    pub(super) fn sim_key_details(&self) -> Result<Vec<PersonaKeys>, Problem> {
        let (login, server) = self.signed_in()?;
        let (_, store, _) = self.open_store()?;
        Ok(server
            .personas
            .iter()
            .filter(|(owner, _, _)| *owner == login)
            .map(|(_, p, published)| {
                let mut rows = vec![("Persona id".to_owned(), p.id.to_string())];
                rows.push((
                    "Identity key".into(),
                    if *published {
                        format!("SHA256:{}", &p.id.simple().to_string()[..16])
                    } else {
                        "none published".into()
                    },
                ));
                if store.keys.contains(&p.id) {
                    rows.push(("KeyPackages waiting".into(), "20".into()));
                }
                PersonaKeys {
                    handle: p.handle.clone(),
                    rows,
                }
            })
            .collect())
    }

    /// Starting over: this computer makes a new identity key, and the
    /// login's other devices lose the persona.
    pub(super) fn sim_start_over(&self, persona: Uuid) -> Result<(), Problem> {
        self.reach()?;
        self.with_server_mut(|server, store| {
            let (_, _, published) = server
                .personas
                .iter_mut()
                .find(|(_, p, _)| p.id == persona)
                .ok_or_else(|| refused("That persona is gone."))?;
            *published = true;
            if !store.keys.contains(&persona) {
                store.keys.push(persona);
            }
            let here = store.here.device;
            for d in &mut server.devices.list {
                if Some(d.id) != here {
                    d.personas.retain(|p| *p != persona);
                }
            }
            Ok(())
        })
    }
}
