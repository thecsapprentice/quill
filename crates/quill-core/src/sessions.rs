//! Sessions (2f) on the client: every device of both writers joins the
//! session's MLS group, reads and writes its log, and keeps what it read.
//!
//! - A device holding the group from the request (the requester's, or an
//!   author device that read the pitch) carries on from its request note.
//! - A device that was in the request's Welcome but never opened it joins
//!   from that Welcome and reads the log from the start.
//! - A device that wasn't in it is added by one of its own persona's
//!   devices already in the group, which leaves a Welcome addressed to it.
//!
//! Every device checks every change against the device directory: members
//! must be devices of the two personas; a commit may only add or remove
//! devices of the committer's own persona. A change that fails stops the
//! device reading the session rather than carry on in a group it can't
//! trust.

use std::collections::{HashMap, HashSet};

pub use crate::history::Message;

use openmls::prelude::{
    BasicCredential, Credential, GroupId, KeyPackage, LeafNodeIndex, LeafNodeParameters, MlsGroup,
    MlsMessageBodyIn, MlsMessageIn, MlsMessageOut, ProcessedMessageContent, ProcessedWelcome,
    Sender, WelcomeError,
};
use openmls_traits::OpenMlsProvider as _;
use quill_proto::{
    events::Event,
    keys::{b64, unb64},
    sessions::{LeaveWelcome, Session, SessionMember},
};
use serde::{Deserialize, Serialize};
use tls_codec::{Deserialize as _, Serialize as _};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    Client, CoreError, Store,
    activity::Activity,
    devices::join_config,
    history::{self, Gap},
    hlc::Hlc,
    keys::{self, DeviceKey, DeviceStatus, PinStatus},
    mls::{DeviceSigner, Provider, mls_err},
    requests::{self, Role},
    store::kv::Table,
};

/// Commits and messages are retried this often after losing an epoch race.
const RETRIES: usize = 5;
/// The channel messages go to unless another is named.
pub const STORY: &str = "story";
/// The longest message, in characters.
pub const MAX_MESSAGE_CHARS: usize = 20_000;

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

fn conflict(e: &CoreError) -> bool {
    e.status() == Some(409)
}

fn encode(m: &MlsMessageOut) -> Result<Vec<u8>, CoreError> {
    m.tls_serialize_detached()
        .map_err(|e| mls_err("encoding", e))
}

fn json<T: Serialize>(v: &T) -> Result<Vec<u8>, CoreError> {
    serde_json::to_vec(v).map_err(|_| err("encoding"))
}

// ---- What this device keeps ----------------------------------------------------------

/// This device's place in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Note {
    request: Uuid,
    /// This login's persona in it.
    persona: Uuid,
    /// The log is read up to here.
    seq: u64,
    /// Messages this device posted that the log hasn't passed yet (MLS
    /// can't decrypt one's own).
    #[serde(default)]
    own: Vec<u64>,
    /// Joined through a last-resort KeyPackage: send a key update.
    #[serde(default)]
    update_due: bool,
    #[serde(default)]
    ended: bool,
    /// Why this device stopped reading the session.
    #[serde(default)]
    broken: Option<String>,
    /// This device's clock in the session.
    #[serde(default)]
    hlc: String,
    /// The last `authorSeq` this device used.
    #[serde(default)]
    author_seq: u64,
    /// How far the server was told this device has read.
    #[serde(default)]
    acked: u64,
}

impl Note {
    fn clock(&self) -> Hlc {
        self.hlc.parse().unwrap_or_default()
    }

    /// Moves the clock past a value read.
    fn saw(&mut self, remote: Hlc) {
        self.hlc = Hlc::read(self.clock(), remote, keys::now_ms()).to_string();
    }
}

fn note(p: &Provider<'_>, gid: &[u8]) -> Result<Option<Note>, CoreError> {
    p.get(Table::Sessions, gid)?
        .map(|v| serde_json::from_slice(&v).map_err(|_| err("a stored session is damaged")))
        .transpose()
}

fn set_note(p: &Provider<'_>, gid: &[u8], n: &Note) -> Result<(), CoreError> {
    p.set(Table::Sessions, gid, Some(json(n)?))
}

fn save_note(store: &Store, gid: &[u8], n: &Note) -> Result<(), CoreError> {
    Provider::new(store).atomically(|p| set_note(p, gid, n))
}

fn load_group(p: &Provider<'_>, gid: &[u8]) -> Result<Option<MlsGroup>, CoreError> {
    MlsGroup::load(p.storage(), &GroupId::from_slice(gid)).map_err(|e| mls_err("loading", e))
}

/// Drops the group's keys, keeping the messages read.
fn drop_group(p: &Provider<'_>, gid: &[u8]) -> Result<(), CoreError> {
    if let Some(mut g) = load_group(p, gid)? {
        g.delete(p.storage())
            .map_err(|e| mls_err("forgetting the session's keys", e))?;
    }
    Ok(())
}

// ---- The directory, as this device judges it -------------------------------------------

/// A persona's devices, checked.
struct Dir {
    uri: String,
    handle: String,
    /// Valid devices: their keys and directory ids.
    valid: HashMap<Vec<u8>, Uuid>,
    /// Devices that verify, or once did (revoked, superseded): a member
    /// may still be one of these until it's removed.
    known: HashSet<Vec<u8>>,
    /// Revoked or superseded: to be removed from sessions.
    stale: HashSet<Vec<u8>>,
}

impl Dir {
    async fn fetch(
        store: &Store,
        client: &Client,
        m: &SessionMember,
        own: bool,
    ) -> Result<Self, CoreError> {
        let checked = keys::check_published(
            &client.actor_keys(&m.handle).await?,
            &m.actor_uri,
            keys::now_ms(),
        )?;
        let (key, version) = checked
            .identity
            .clone()
            .ok_or_else(|| err(format!("@{} has no identity key", m.handle)))?;
        if own {
            let persona = m.persona.ok_or_else(|| err("not this login's persona"))?;
            let mine = keys::load_identity(store, persona)?
                .ok_or_else(|| requests::not_set_up(&m.handle))?;
            if mine.public().as_slice() != key.as_slice() {
                return Err(err(format!(
                    "the server publishes another identity key for @{}",
                    m.handle
                )));
            }
        } else if let PinStatus::Changed { .. } =
            keys::check_pin(store, &m.actor_uri, &key, version)?
        {
            return Err(err(format!(
                "@{}'s identity key changed since it was pinned; check it with `quill keys show {}`",
                m.handle, m.handle
            )));
        }
        let mut dir = Self {
            uri: m.actor_uri.clone(),
            handle: m.handle.clone(),
            valid: HashMap::new(),
            known: HashSet::new(),
            stale: HashSet::new(),
        };
        for d in checked.devices {
            match d.status {
                DeviceStatus::Valid { .. } => {
                    dir.valid.insert(d.device_key.clone(), d.id);
                    dir.known.insert(d.device_key);
                }
                DeviceStatus::Revoked(_) | DeviceStatus::Superseded => {
                    dir.stale.insert(d.device_key.clone());
                    dir.known.insert(d.device_key);
                }
                DeviceStatus::Invalid(_) => {}
            }
        }
        Ok(dir)
    }
}

fn identity(c: &Credential) -> Option<String> {
    let basic = BasicCredential::try_from(c.clone()).ok()?;
    String::from_utf8(basic.identity().to_vec()).ok()
}

/// The two personas' directories.
struct Dirs {
    me: Dir,
    partner: Dir,
}

impl Dirs {
    fn of(&self, uri: &str) -> Option<&Dir> {
        [&self.me, &self.partner].into_iter().find(|d| d.uri == uri)
    }

    /// Every member is a known device of one of the two personas.
    fn check_members(&self, group: &MlsGroup) -> Result<(), CoreError> {
        for m in group.members() {
            let ok = identity(&m.credential)
                .and_then(|uri| self.of(&uri))
                .is_some_and(|d| d.known.contains(&m.signature_key));
            if !ok {
                return Err(err(
                    "the session holds a device that belongs to neither writer",
                ));
            }
        }
        Ok(())
    }
}

// ---- Joining -------------------------------------------------------------------------------

enum Joined {
    /// Joined; whether through a last-resort KeyPackage.
    Yes { last_resort: bool },
    /// The Welcome isn't for this device.
    NotForMe,
}

fn join(p: &Provider<'_>, welcome: &str, gid: &[u8], dirs: &Dirs) -> Result<Joined, CoreError> {
    let bytes = unb64(welcome).ok_or_else(|| err("server sent bad base64"))?;
    let welcome = match MlsMessageIn::tls_deserialize_exact(bytes)
        .map_err(|e| mls_err("decoding the Welcome", e))?
        .extract()
    {
        MlsMessageBodyIn::Welcome(w) => w,
        _ => return Err(err("not a Welcome")),
    };
    let processed = match ProcessedWelcome::new_from_welcome(p, &join_config(), welcome) {
        Ok(w) => w,
        Err(WelcomeError::NoMatchingKeyPackage) => return Ok(Joined::NotForMe),
        Err(e) => return Err(mls_err("joining the session", e)),
    };
    let last_resort = processed
        .own_key_package()
        .is_some_and(KeyPackage::last_resort);
    let staged = processed
        .into_staged_welcome(p, None)
        .map_err(|e| mls_err("joining the session", e))?;
    if staged.group_context().group_id().as_slice() != gid {
        return Err(err("the Welcome is for another group"));
    }
    let group = staged
        .into_group(p)
        .map_err(|e| mls_err("joining the session", e))?;
    dirs.check_members(&group)?;
    Ok(Joined::Yes { last_resort })
}

// ---- Reading the log -------------------------------------------------------------------------

/// What applying one logged message did.
#[derive(Default)]
struct Applied {
    /// This device was removed.
    removed: bool,
    /// The clock value of an activity read.
    clock: Option<Hlc>,
}

/// Applies one logged message within the caller's batch.
fn apply(
    p: &Provider<'_>,
    gid: &[u8],
    seq: u64,
    bytes: &[u8],
    dirs: &Dirs,
) -> Result<Applied, CoreError> {
    let mut group = load_group(p, gid)?.ok_or_else(|| err("the session's keys are missing"))?;
    let message = MlsMessageIn::tls_deserialize_exact(bytes)
        .map_err(|e| mls_err("decoding", e))?
        .try_into_protocol_message()
        .map_err(|e| mls_err("decoding", e))?;
    let processed = group
        .process_message(p, message)
        .map_err(|e| mls_err("processing", e))?;
    let Sender::Member(index) = *processed.sender() else {
        return Err(err("a message from outside the group"));
    };
    let sender = identity(processed.credential()).ok_or_else(|| err("an unreadable sender"))?;
    let sender_dir = dirs
        .of(&sender)
        .ok_or_else(|| err("a message from neither writer"))?;
    let sender_key = group
        .member_at(index)
        .map(|m| m.signature_key)
        .ok_or_else(|| err("the sender isn't in the group"))?;
    if !sender_dir.known.contains(&sender_key) {
        return Err(err("a message from a device the directory doesn't list"));
    }
    match processed.into_content() {
        ProcessedMessageContent::ApplicationMessage(app) => {
            let plain = Zeroizing::new(app.into_bytes());
            let a: Activity =
                serde_json::from_slice(&plain).map_err(|_| err("an unreadable message"))?;
            history::record(
                p,
                gid,
                history::From {
                    seq,
                    sender: &sender,
                    device: &b64(&sender_key),
                    own: false,
                },
                &a,
            )?;
            Ok(Applied {
                removed: false,
                clock: Some(a.clock()),
            })
        }
        ProcessedMessageContent::StagedCommitMessage(staged) => {
            // A writer adds and removes only their own persona's devices.
            for add in staged.add_proposals() {
                let leaf = add.add_proposal().key_package().leaf_node();
                let ok = identity(leaf.credential()).as_deref() == Some(sender.as_str())
                    && sender_dir
                        .valid
                        .contains_key(leaf.signature_key().as_slice());
                if !ok {
                    return Err(err(
                        "a commit added a device that isn't one of the committer's own: refused",
                    ));
                }
            }
            for remove in staged.remove_proposals() {
                let removed = remove.remove_proposal().removed();
                let whose = group.member(removed).and_then(identity);
                if whose.as_deref() != Some(sender.as_str()) {
                    return Err(err(
                        "a commit removed a device that isn't one of the committer's own: refused",
                    ));
                }
            }
            if staged.update_proposals().next().is_some() {
                return Err(err("a commit with update proposals: refused"));
            }
            if let Some(leaf) = staged.update_path_leaf_node() {
                let ok = identity(leaf.credential()).as_deref() == Some(sender.as_str())
                    && sender_dir.known.contains(leaf.signature_key().as_slice());
                if !ok {
                    return Err(err("a commit changed its sender's key: refused"));
                }
            }
            let removed = staged.self_removed();
            group
                .merge_staged_commit(p, *staged)
                .map_err(|e| mls_err("merging", e))?;
            if !removed {
                dirs.check_members(&group)?;
            }
            Ok(Applied {
                removed,
                clock: None,
            })
        }
        // Quill sends no standalone proposals.
        _ => Err(err("an unexpected proposal")),
    }
}

/// Reads everything new in the session's log. Returns how many messages
/// it read, and whether this device was removed.
async fn catch_up(
    store: &Store,
    client: &Client,
    gid: &[u8],
    dirs: &Dirs,
) -> Result<(usize, bool), CoreError> {
    let gid_text = b64(gid);
    let mut read = 0;
    loop {
        let after = {
            let p = Provider::new(store);
            note(&p, gid)?.map_or(0, |n| n.seq)
        };
        let page = client
            .group_messages(
                &gid_text,
                &quill_proto::groups::MessagesQuery {
                    after: Some(after),
                    limit: Some(100),
                },
            )
            .await?;
        for m in &page.messages {
            let bytes = unb64(&m.message).ok_or_else(|| err("server sent bad base64"))?;
            let provider = Provider::new(store);
            let removed = provider.atomically(|p| {
                let mut n = note(p, gid)?.ok_or_else(|| err("session state missing"))?;
                let applied = if n.own.contains(&m.seq) {
                    Applied::default()
                } else {
                    apply(p, gid, m.seq, &bytes, dirs)?
                };
                if let Some(clock) = applied.clock {
                    n.saw(clock);
                }
                let removed = applied.removed;
                n.own.retain(|s| *s > m.seq);
                n.seq = m.seq;
                if removed {
                    n.ended = true;
                    n.broken = Some("this device was removed from the session".into());
                    drop_group(p, gid)?;
                }
                set_note(p, gid, &n)?;
                Ok(removed)
            });
            let removed = match removed {
                Ok(r) => r,
                Err(e) => {
                    // Stop reading this session: something about it can't
                    // be trusted, or this device can't follow it.
                    let p = Provider::new(store);
                    if let Some(mut n) = note(&p, gid)? {
                        n.broken = Some(format!("message {}: {e}", m.seq));
                        save_note(store, gid, &n)?;
                    }
                    return Err(e);
                }
            };
            read += 1;
            if removed {
                return Ok((read, true));
            }
        }
        if !page.more {
            return Ok((read, false));
        }
    }
}

// ---- Changing the group --------------------------------------------------------------------

/// This device in a session.
struct Me {
    device: DeviceKey,
    uri: String,
}

impl Me {
    fn signer(&self) -> DeviceSigner<'_> {
        DeviceSigner(self.device.signing_key())
    }
}

/// Notes a message this device posted.
fn record_own(store: &Store, gid: &[u8], seq: u64) -> Result<(), CoreError> {
    Provider::new(store).atomically(|p| {
        let mut n = note(p, gid)?.ok_or_else(|| err("session state missing"))?;
        n.own.push(seq);
        set_note(p, gid, &n)
    })
}

/// Posts a commit made by `make`, merging it once the server takes it.
/// Losing an epoch race clears it, catches up, and asks `make` again
/// (which may now find nothing to do).
async fn commit<T>(
    store: &Store,
    client: &Client,
    gid: &[u8],
    dirs: &Dirs,
    make: impl Fn(&Provider<'_>, &mut MlsGroup) -> Result<Option<(MlsMessageOut, T)>, CoreError>,
) -> Result<Option<(u64, T)>, CoreError> {
    for _ in 0..RETRIES {
        let provider = Provider::new(store);
        let mut group =
            load_group(&provider, gid)?.ok_or_else(|| err("the session's keys are missing"))?;
        let Some((message, extra)) = provider.atomically(|p| make(p, &mut group))? else {
            return Ok(None);
        };
        let body = quill_proto::groups::PostMessage {
            message: b64(&encode(&message)?),
        };
        let clear = |provider: &Provider<'_>, group: &mut MlsGroup| {
            provider.atomically(|p| {
                group
                    .clear_pending_commit(p.storage())
                    .map_err(|e| mls_err("clearing", e))
            })
        };
        match client.post_group_message(&b64(gid), &body).await {
            Ok(posted) => {
                provider.atomically(|p| {
                    group
                        .merge_pending_commit(p)
                        .map_err(|e| mls_err("merging", e))
                })?;
                record_own(store, gid, posted.seq)?;
                return Ok(Some((posted.seq, extra)));
            }
            Err(e) if conflict(&e) => {
                clear(&provider, &mut group)?;
                catch_up(store, client, gid, dirs).await?;
            }
            Err(e) => {
                let _ = clear(&provider, &mut group);
                return Err(e);
            }
        }
    }
    Err(err("the session kept changing; try again"))
}

/// Brings this persona's devices in the group in line with the directory:
/// adds valid devices that aren't in it, removes revoked ones. Returns
/// `(added, removed)`.
async fn tend(
    store: &Store,
    client: &Client,
    gid: &[u8],
    dirs: &Dirs,
    me: &Me,
) -> Result<(usize, usize), CoreError> {
    let members = |store: &Store| -> Result<Vec<(LeafNodeIndex, Vec<u8>)>, CoreError> {
        let p = Provider::new(store);
        let g = load_group(&p, gid)?.ok_or_else(|| err("the session's keys are missing"))?;
        Ok(g.members()
            .filter(|m| identity(&m.credential).as_deref() == Some(me.uri.as_str()))
            .map(|m| (m.index, m.signature_key))
            .collect())
    };
    let mine = members(store)?;
    let stale: Vec<Vec<u8>> = mine
        .iter()
        .filter(|(_, k)| dirs.me.stale.contains(k) || !dirs.me.known.contains(k))
        .map(|(_, k)| k.clone())
        .collect();
    let mut removed = 0;
    if !stale.is_empty() {
        let done = commit(store, client, gid, dirs, |p, group| {
            let leaves: Vec<LeafNodeIndex> = group
                .members()
                .filter(|m| identity(&m.credential).as_deref() == Some(me.uri.as_str()))
                .filter(|m| stale.contains(&m.signature_key))
                .map(|m| m.index)
                .collect();
            if leaves.is_empty() {
                return Ok(None);
            }
            let (commit, _, _) = group
                .remove_members(p, &me.signer(), &leaves)
                .map_err(|e| mls_err("removing a device", e))?;
            Ok(Some((commit, leaves.len())))
        })
        .await?;
        removed = done.map_or(0, |(_, n)| n);
    }

    let present: HashSet<Vec<u8>> = members(store)?.into_iter().map(|(_, k)| k).collect();
    let missing: Vec<Uuid> = dirs
        .me
        .valid
        .iter()
        .filter(|(k, _)| !present.contains(*k))
        .map(|(_, id)| *id)
        .collect();
    if missing.is_empty() {
        return Ok((0, removed));
    }
    let mut packages: Vec<(Uuid, KeyPackage)> = Vec::new();
    for device in missing {
        let claim = keys::claim_for(store, client, &dirs.me.handle, &me.uri, Some(device)).await?;
        packages.extend(
            claim
                .packages
                .into_iter()
                .filter(|p| p.device == device)
                .map(|p| (p.device, p.key_package)),
        );
    }
    if packages.is_empty() {
        return Ok((0, removed));
    }
    let done = commit(store, client, gid, dirs, |p, group| {
        let present: HashSet<Vec<u8>> = group.members().map(|m| m.signature_key).collect();
        let (ids, adds): (Vec<Uuid>, Vec<KeyPackage>) = packages
            .iter()
            .filter(|(_, k)| !present.contains(k.leaf_node().signature_key().as_slice()))
            .map(|(id, k)| (*id, k.clone()))
            .unzip();
        if adds.is_empty() {
            return Ok(None);
        }
        let (commit, welcome, _) = group
            .add_members(p, &me.signer(), &adds)
            .map_err(|e| mls_err("adding a device", e))?;
        Ok(Some((commit, (ids, encode(&welcome)?))))
    })
    .await?;
    let Some((seq, (ids, welcome))) = done else {
        return Ok((0, removed));
    };
    for device in &ids {
        client
            .leave_welcome(
                &b64(gid),
                &LeaveWelcome {
                    device: *device,
                    welcome: b64(&welcome),
                    from_seq: seq,
                },
            )
            .await?;
    }
    Ok((ids.len(), removed))
}

/// Sends a key update, after joining through a last-resort KeyPackage.
async fn update_key(
    store: &Store,
    client: &Client,
    gid: &[u8],
    dirs: &Dirs,
    me: &Me,
) -> Result<(), CoreError> {
    commit(store, client, gid, dirs, |p, group| {
        let bundle = group
            .self_update(p, &me.signer(), LeafNodeParameters::default())
            .map_err(|e| mls_err("updating this device's key", e))?;
        Ok(Some((bundle.into_commit(), ())))
    })
    .await?;
    Ok(())
}

// ---- Syncing ---------------------------------------------------------------------------------

/// What [`sync`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// Sessions this device joined.
    pub joined: usize,
    /// Messages read.
    pub read: usize,
    /// This persona's devices added to sessions, and removed.
    pub added: usize,
    pub removed: usize,
    /// Sessions this device couldn't sync, by id, with why.
    pub problems: Vec<(String, String)>,
}

/// This device's standing in a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// In the group and reading.
    Joined,
    /// Waiting to be added by another of this persona's devices.
    Waiting,
    /// This persona has no keys on this device.
    NoKeys,
    /// Stopped reading, with why.
    Broken(String),
}

/// One of the login's sessions, as this device sees it.
#[derive(Debug, Clone)]
pub struct Listed {
    pub session: Session,
    pub standing: Standing,
    /// Messages this device holds.
    pub messages: usize,
}

impl Listed {
    /// This login's persona in it.
    pub fn me(&self) -> Option<&SessionMember> {
        self.session.members.iter().find(|m| m.persona.is_some())
    }

    /// The other writer, while they're still in it.
    pub fn partner(&self) -> Option<&SessionMember> {
        self.session.members.iter().find(|m| m.persona.is_none())
    }
}

fn this_device(store: &Store, s: &Session) -> Result<Option<(SessionMember, Me)>, CoreError> {
    let Some(mine) = s.members.iter().find(|m| m.persona.is_some()) else {
        return Ok(None);
    };
    let persona = mine.persona.unwrap_or_default();
    let Some(device) = keys::load_device(store, persona)?.filter(|d| d.id.is_some()) else {
        return Ok(None);
    };
    Ok(Some((
        mine.clone(),
        Me {
            device,
            uri: mine.actor_uri.clone(),
        },
    )))
}

/// Syncs every session of the login: joins, reads, keeps this persona's
/// devices in line. Problems with one session don't stop the others.
pub async fn sync(store: &Store, client: &Client) -> Result<SyncReport, CoreError> {
    let mut report = SyncReport::default();
    for s in client.sessions().await? {
        if let Err(e) = sync_one(store, client, &s, &mut report).await {
            report.problems.push((s.id.clone(), e.to_string()));
        }
    }
    Ok(report)
}

async fn sync_one(
    store: &Store,
    client: &Client,
    s: &Session,
    report: &mut SyncReport,
) -> Result<Standing, CoreError> {
    let gid = unb64(&s.id).ok_or_else(|| err("server sent bad base64"))?;
    let current = note(&Provider::new(store), &gid)?;
    if let Some(n) = &current {
        if let Some(why) = &n.broken {
            return Ok(Standing::Broken(why.clone()));
        }
        if n.ended {
            return Ok(Standing::Joined);
        }
    }
    let Some((mine, me)) = this_device(store, s)? else {
        return Ok(Standing::NoKeys);
    };
    let Some(partner) = s.members.iter().find(|m| m.persona.is_none()) else {
        // The other writer's persona is gone: the session has ended.
        return end_here(store, client, s, &gid, current, None).await;
    };
    let dirs = Dirs {
        me: Dir::fetch(store, client, &mine, true).await?,
        partner: Dir::fetch(store, client, partner, false).await?,
    };
    if current.is_none() && !start(store, client, s, &gid, &mine, &me, &dirs, report).await? {
        return Ok(Standing::Waiting);
    }
    if s.ended {
        return end_here(store, client, s, &gid, None, Some((&dirs, &me))).await;
    }
    let (read, removed) = catch_up(store, client, &gid, &dirs).await?;
    report.read += read;
    if removed {
        return Ok(Standing::Broken(
            "this device was removed from the session".into(),
        ));
    }
    acknowledge(store, client, s, &gid, &me).await?;
    let due = note(&Provider::new(store), &gid)?.is_some_and(|n| n.update_due);
    if due {
        update_key(store, client, &gid, &dirs, &me).await?;
        Provider::new(store).atomically(|p| {
            let mut n = note(p, &gid)?.ok_or_else(|| err("session state missing"))?;
            n.update_due = false;
            set_note(p, &gid, &n)
        })?;
    }
    let (added, gone) = tend(store, client, &gid, &dirs, &me).await?;
    report.added += added;
    report.removed += gone;
    Ok(Standing::Joined)
}

/// Starts following a session on this device: from its request note, or
/// by joining from a Welcome. Returns false when no Welcome is for it yet.
#[allow(clippy::too_many_arguments)]
async fn start(
    store: &Store,
    client: &Client,
    s: &Session,
    gid: &[u8],
    mine: &SessionMember,
    me: &Me,
    dirs: &Dirs,
    report: &mut SyncReport,
) -> Result<bool, CoreError> {
    let persona = mine.persona.unwrap_or_default();
    let fresh = |seq: u64, update_due: bool| Note {
        request: s.request,
        persona,
        seq,
        own: Vec::new(),
        update_due,
        ended: false,
        broken: None,
        hlc: String::new(),
        author_seq: 0,
        acked: 0,
    };
    // From the request: the group is here already, and the pitch (seq 1)
    // was read or sent by this device.
    if let Some(held) = requests::held(store, s.request)? {
        let provider = Provider::new(store);
        if held.group_id == s.id && load_group(&provider, gid)?.is_some() {
            provider.atomically(|p| {
                let mut n = fresh(1, false);
                if let Some(pitch) = &held.pitch {
                    let (sender, own) = match held.role {
                        Role::Outgoing => (me.uri.clone(), true),
                        Role::Incoming => (held.requester.clone().unwrap_or_default(), false),
                    };
                    history::record(
                        p,
                        gid,
                        history::From {
                            seq: 1,
                            sender: &sender,
                            device: &pitch.device,
                            own,
                        },
                        &pitch.activity,
                    )?;
                    n.saw(pitch.activity.clock());
                    if own {
                        n.author_seq = pitch.activity.author_seq.unwrap_or(0);
                    }
                }
                set_note(p, gid, &n)?;
                p.set(Table::Requests, s.request.as_bytes(), None)
            })?;
            return Ok(true);
        }
    }
    // A Welcome addressed to this device, then the request's.
    let my_id = me.device.id.unwrap_or_default();
    let mut tries: Vec<(Option<Uuid>, &str, u64)> = s
        .device_welcomes
        .iter()
        .filter(|w| w.device == my_id)
        .map(|w| (Some(w.device), w.welcome.as_str(), w.from_seq))
        .collect();
    tries.push((None, s.welcome.as_str(), 0));
    for (device, welcome, from) in tries {
        let provider = Provider::new(store);
        let joined = provider.atomically(|p| {
            let j = join(p, welcome, gid, dirs)?;
            if let Joined::Yes { last_resort } = j {
                set_note(p, gid, &fresh(from, last_resort))?;
            }
            Ok(j)
        })?;
        if let Joined::Yes { .. } = joined {
            if let Some(d) = device {
                client.take_welcome(&s.id, d).await?;
            }
            report.joined += 1;
            return Ok(true);
        }
    }
    Ok(false)
}

/// The session ended: read to the end (if this device can), then drop the
/// group's keys, keeping the messages.
async fn end_here(
    store: &Store,
    client: &Client,
    s: &Session,
    gid: &[u8],
    current: Option<Note>,
    reader: Option<(&Dirs, &Me)>,
) -> Result<Standing, CoreError> {
    let joined = current.is_some() || note(&Provider::new(store), gid)?.is_some();
    if !joined {
        return Ok(Standing::Joined);
    }
    if let Some((dirs, me)) = reader {
        catch_up(store, client, gid, dirs).await?;
        acknowledge(store, client, s, gid, me).await?;
    }
    Provider::new(store).atomically(|p| {
        let mut n = note(p, gid)?.ok_or_else(|| err("session state missing"))?;
        n.ended = true;
        drop_group(p, gid)?;
        set_note(p, gid, &n)
    })?;
    Ok(Standing::Joined)
}

// ---- Commands --------------------------------------------------------------------------------

/// Finds a session by its id or an unambiguous prefix of it.
pub async fn find(client: &Client, id: &str) -> Result<Session, CoreError> {
    let id = id.trim();
    if id.len() < 6 {
        return Err(err("give at least 6 characters of the session id"));
    }
    let mut hits: Vec<Session> = client
        .sessions()
        .await?
        .into_iter()
        .filter(|s| s.id.starts_with(id))
        .collect();
    match hits.len() {
        0 => Err(err(format!("no session {id}"))),
        1 => Ok(hits.remove(0)),
        _ => Err(err(format!("{id} matches more than one session"))),
    }
}

/// The login's sessions after syncing them all.
pub async fn list(store: &Store, client: &Client) -> Result<Vec<Listed>, CoreError> {
    let mut report = SyncReport::default();
    let mut out = Vec::new();
    for s in client.sessions().await? {
        let standing = match sync_one(store, client, &s, &mut report).await {
            Ok(st) => st,
            Err(e) => Standing::Broken(e.to_string()),
        };
        let gid = unb64(&s.id).unwrap_or_default();
        out.push(Listed {
            messages: history::count(store, &gid)?,
            standing,
            session: s,
        });
    }
    Ok(out)
}

/// A session as this device holds it.
#[derive(Debug, Clone)]
pub struct Reading {
    pub session: Session,
    /// In display order.
    pub messages: Vec<Message>,
    /// Activities the server seems to have held back.
    pub gaps: Vec<Gap>,
}

fn standing_error(standing: Standing) -> Result<(), CoreError> {
    match standing {
        Standing::Joined => Ok(()),
        Standing::Waiting => Err(err(
            "this device isn't in the session yet: another of your devices for this persona adds it when it next runs",
        )),
        Standing::NoKeys => Err(err("this persona has no keys on this device")),
        Standing::Broken(why) => Err(err(why)),
    }
}

/// A session's notes as this device holds them, after syncing it.
pub async fn read(store: &Store, client: &Client, id: &str) -> Result<Reading, CoreError> {
    let s = find(client, id).await?;
    let mut report = SyncReport::default();
    let standing = sync_one(store, client, &s, &mut report).await?;
    let gid = unb64(&s.id).ok_or_else(|| err("bad session id"))?;
    let messages = history::messages(store, &gid)?;
    if messages.is_empty() {
        standing_error(standing)?;
    }
    Ok(Reading {
        gaps: history::gaps(store, &gid)?,
        messages,
        session: s,
    })
}

/// Tells the server how far this device has read, so it can delete what
/// every device has. Only the server sees it, never the other writer.
async fn acknowledge(
    store: &Store,
    client: &Client,
    s: &Session,
    gid: &[u8],
    me: &Me,
) -> Result<(), CoreError> {
    let Some(n) = note(&Provider::new(store), gid)? else {
        return Ok(());
    };
    let Some(device) = me.device.id else {
        return Ok(());
    };
    if n.seq <= n.acked {
        return Ok(());
    }
    client
        .ack_session(&s.id, &quill_proto::sessions::Ack { device, seq: n.seq })
        .await?;
    Provider::new(store).atomically(|p| {
        let mut n = note(p, gid)?.ok_or_else(|| err("session state missing"))?;
        n.acked = n.seq;
        set_note(p, gid, &n)
    })
}

/// A session this device can send to, synced.
struct Ready {
    session: Session,
    gid: Vec<u8>,
    me: Me,
    dirs: Dirs,
}

async fn ready(store: &Store, client: &Client, id: &str) -> Result<Ready, CoreError> {
    let s = find(client, id).await?;
    if s.ended {
        return Err(err("the session has ended"));
    }
    let mut report = SyncReport::default();
    standing_error(sync_one(store, client, &s, &mut report).await?)?;
    let gid = unb64(&s.id).ok_or_else(|| err("bad session id"))?;
    let (mine, me) = this_device(store, &s)?.ok_or_else(|| err("no keys on this device"))?;
    let partner = s
        .members
        .iter()
        .find(|m| m.persona.is_none())
        .ok_or_else(|| err("the session has ended"))?;
    let dirs = Dirs {
        me: Dir::fetch(store, client, &mine, true).await?,
        partner: Dir::fetch(store, client, partner, false).await?,
    };
    Ok(Ready {
        session: s,
        gid,
        me,
        dirs,
    })
}

/// Sends the activity `make` builds from this device's next clock value
/// and `authorSeq`, and keeps it. Returns its place in the log.
async fn send(
    store: &Store,
    client: &Client,
    r: &Ready,
    make: impl Fn(Hlc, u64) -> Activity,
) -> Result<u64, CoreError> {
    let (hlc, author_seq) = {
        let n = note(&Provider::new(store), &r.gid)?.ok_or_else(|| err("session state missing"))?;
        (Hlc::send(n.clock(), keys::now_ms()), n.author_seq + 1)
    };
    // The same activity on every retry, so a lost race leaves no gap.
    let activity = make(hlc, author_seq);
    let plain = Zeroizing::new(json(&activity)?);
    let device = b64(&r.me.device.public());
    for _ in 0..RETRIES {
        let provider = Provider::new(store);
        let message = provider.atomically(|p| {
            let mut group =
                load_group(p, &r.gid)?.ok_or_else(|| err("the session's keys are missing"))?;
            group
                .create_message(p, &r.me.signer(), &plain)
                .map_err(|e| mls_err("encrypting", e))
        })?;
        let body = quill_proto::groups::PostMessage {
            message: b64(&encode(&message)?),
        };
        match client.post_group_message(&r.session.id, &body).await {
            Ok(posted) => {
                Provider::new(store).atomically(|p| {
                    history::record(
                        p,
                        &r.gid,
                        history::From {
                            seq: posted.seq,
                            sender: &r.me.uri,
                            device: &device,
                            own: true,
                        },
                        &activity,
                    )?;
                    let mut n = note(p, &r.gid)?.ok_or_else(|| err("session state missing"))?;
                    n.own.push(posted.seq);
                    n.saw(hlc);
                    n.author_seq = n.author_seq.max(author_seq);
                    set_note(p, &r.gid, &n)
                })?;
                return Ok(posted.seq);
            }
            Err(e) if conflict(&e) => {
                catch_up(store, client, &r.gid, &r.dirs).await?;
            }
            Err(e) => return Err(e),
        }
    }
    Err(err("the session kept changing; try again"))
}

fn check_text(text: &str) -> Result<&str, CoreError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(err("nothing to send"));
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(err(format!(
            "too long (at most {MAX_MESSAGE_CHARS} characters)"
        )));
    }
    Ok(text)
}

/// Writes a message to a session. Returns its place in the log.
pub async fn write(
    store: &Store,
    client: &Client,
    id: &str,
    text: &str,
    channel: Option<&str>,
) -> Result<u64, CoreError> {
    let text = check_text(text)?;
    let channel = channel.unwrap_or(STORY);
    let r = ready(store, client, id).await?;
    send(store, client, &r, |hlc, n| {
        Activity::create(text, channel, hlc, n)
    })
    .await
}

/// One of this persona's notes, to change.
fn own_note(store: &Store, r: &Ready, at: u64) -> Result<Message, CoreError> {
    let m = history::message(store, &r.gid, at)?
        .ok_or_else(|| err(format!("no message #{at} on this device")))?;
    if m.sender != r.me.uri {
        return Err(err("only its author can change a message"));
    }
    if m.deleted {
        return Err(err("that message was deleted"));
    }
    Ok(m)
}

/// Replaces the text of message `at` (the log position of its first
/// version). Every version is kept.
pub async fn edit(
    store: &Store,
    client: &Client,
    id: &str,
    at: u64,
    text: &str,
) -> Result<u64, CoreError> {
    let text = check_text(text)?;
    let r = ready(store, client, id).await?;
    let m = own_note(store, &r, at)?;
    send(store, client, &r, |hlc, n| {
        Activity::update(&m.id, text, &m.channel, hlc, n)
    })
    .await
}

/// Deletes message `at`: it shows as deleted; its versions are kept.
pub async fn delete(store: &Store, client: &Client, id: &str, at: u64) -> Result<u64, CoreError> {
    let r = ready(store, client, id).await?;
    let m = own_note(store, &r, at)?;
    send(store, client, &r, |hlc, n| {
        Activity::delete(&m.id, &m.channel, hlc, n)
    })
    .await
}

/// Message `at` with every version, after syncing the session.
pub async fn versions(
    store: &Store,
    client: &Client,
    id: &str,
    at: u64,
) -> Result<Message, CoreError> {
    let reading = read(store, client, id).await?;
    let gid = unb64(&reading.session.id).ok_or_else(|| err("bad session id"))?;
    history::message(store, &gid, at)?
        .ok_or_else(|| err(format!("no message #{at} on this device")))
}

/// Reads a session, then stays on the live event stream, calling `show`
/// with each batch of notes that are new or changed. Returns when the
/// session ends, the stream closes, or `show` returns false.
pub async fn follow(
    store: &Store,
    client: &Client,
    id: &str,
    mut show: impl FnMut(&Reading, &[Message]) -> bool,
) -> Result<(), CoreError> {
    // Open the stream first, so nothing sent while reading is missed.
    let mut events = client.events().await?;
    let mut shown: Vec<Message> = Vec::new();
    loop {
        let reading = read(store, client, id).await?;
        let fresh: Vec<Message> = reading
            .messages
            .iter()
            .filter(|m| !shown.contains(m))
            .cloned()
            .collect();
        if !fresh.is_empty() && !show(&reading, &fresh) {
            return Ok(());
        }
        shown = reading.messages.clone();
        if reading.session.ended {
            return Ok(());
        }
        loop {
            match events.next().await? {
                None => return Ok(()),
                Some(Event::Message { group, .. }) if group == reading.session.id => break,
                Some(Event::Session { id, .. }) if id == reading.session.id => break,
                Some(Event::Resync) => break,
                Some(_) => {}
            }
        }
    }
}

/// Leaves a session, which ends it for both writers. This device reads to
/// the end first and keeps what it read.
pub async fn leave(store: &Store, client: &Client, id: &str) -> Result<(), CoreError> {
    let s = find(client, id).await?;
    let mut report = SyncReport::default();
    // Read what's there first; a problem doesn't stop leaving.
    let _ = sync_one(store, client, &s, &mut report).await;
    client.leave_session(&s.id).await?;
    let gid = unb64(&s.id).ok_or_else(|| err("bad session id"))?;
    Provider::new(store).atomically(|p| {
        drop_group(p, &gid)?;
        if let Some(mut n) = note(p, &gid)? {
            n.ended = true;
            set_note(p, &gid, &n)?;
        }
        Ok(())
    })
}

/// The partner's handle for a sender URI, or the URI itself.
pub fn sender_name(s: &Session, uri: &str) -> String {
    s.members
        .iter()
        .find(|m| m.actor_uri == uri)
        .map_or_else(|| uri.to_owned(), |m| format!("@{}", m.handle))
}

/// Tools for tests that play a dishonest writer. Not built into clients.
#[cfg(feature = "test-tools")]
pub mod testing {
    use super::*;

    /// Commits an add of a persona's devices to a session with none of the
    /// checks an honest client makes, as a dishonest writer's client could.
    pub async fn forge_add(
        store: &Store,
        client: &Client,
        id: &str,
        handle: &str,
        uri: &str,
    ) -> Result<u64, CoreError> {
        let s = find(client, id).await?;
        let gid = unb64(&s.id).ok_or_else(|| err("bad session id"))?;
        let (_, me) = this_device(store, &s)?.ok_or_else(|| err("no keys on this device"))?;
        let claim = keys::claim(store, client, handle, uri).await?;
        let provider = Provider::new(store);
        let mut group =
            load_group(&provider, &gid)?.ok_or_else(|| err("the session's keys are missing"))?;
        let present: HashSet<Vec<u8>> = group.members().map(|m| m.signature_key).collect();
        let adds: Vec<KeyPackage> = claim
            .packages
            .into_iter()
            .map(|p| p.key_package)
            .filter(|k| !present.contains(k.leaf_node().signature_key().as_slice()))
            .collect();
        let message = provider.atomically(|p| {
            let (c, _, _) = group
                .add_members(p, &me.signer(), &adds)
                .map_err(|e| mls_err("adding", e))?;
            Ok(c)
        })?;
        let posted = client
            .post_group_message(
                &s.id,
                &quill_proto::groups::PostMessage {
                    message: b64(&encode(&message)?),
                },
            )
            .await?;
        provider.atomically(|p| {
            group
                .merge_pending_commit(p)
                .map_err(|e| mls_err("merging", e))
        })?;
        record_own(store, &gid, posted.seq)?;
        Ok(posted.seq)
    }
}
