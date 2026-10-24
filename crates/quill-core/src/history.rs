//! A session's messages as this device keeps them (2g): every version of
//! every note, in the order the writers' clocks give them.
//!
//! - A `Create` adds a note. An `Update` adds a version and a `Delete`
//!   hides it; both count only from the note's own author (the same
//!   persona). Nothing is ever removed: every version stays.
//! - Every activity's `authorSeq` is noted per sending device, so a gap
//!   (a number missing between the first and last seen) shows the server
//!   held something back.
//!
//! Keys in [`Table::Messages`], each starting with the group id's length
//! and the group id:
//! - `0x00` + log position: a note, by the position of its `Create`;
//! - `0x01` + note id: the position of the note's `Create`;
//! - `0x02` + device key: the `authorSeq` values seen from that device;
//! - `0x03` + channel name: a channel and every description it has had;
//! - `0x04` + channel name: the last note position this device showed in
//!   it (unread counts are kept here only, never sent anywhere);
//! - `0x05` + blob URL: an attachment still to fetch (2i).
//!
//! Channels (2h) live only inside the encrypted activities. `story` and
//! `planning` always exist; naming a channel (`Create` of a `wr:Channel`)
//! or writing in one makes it exist. A malformed channel name files the
//! note under `story`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use quill_proto::validate::{self, DEFAULT_CHANNELS};

use crate::{
    CoreError, Store, activity::Activity, attachments::Attachment, hlc::Hlc, mls::Provider,
    store::kv::Table,
};

const NOTE: u8 = 0;
const INDEX: u8 = 1;
const SEEN: u8 = 2;
const CHANNEL: u8 = 3;
const SHOWN: u8 = 4;
const PENDING: u8 = 5;

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

fn prefix(gid: &[u8], kind: u8) -> Vec<u8> {
    let mut k = Vec::with_capacity(gid.len() + 2);
    k.push(u8::try_from(gid.len()).unwrap_or(u8::MAX));
    k.extend_from_slice(gid);
    k.push(kind);
    k
}

fn key(gid: &[u8], kind: u8, rest: &[u8]) -> Vec<u8> {
    let mut k = prefix(gid, kind);
    k.extend_from_slice(rest);
    k
}

/// One version of a note: its text, or `None` where it was deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// Its place in the session's log.
    pub seq: u64,
    pub hlc: String,
    #[serde(default)]
    pub content: Option<String>,
}

/// A note as this device keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// The log position of its `Create`, which also names it to users.
    pub seq: u64,
    /// The note's id (`urn:uuid:...`).
    pub id: String,
    /// The author's actor URI.
    pub sender: String,
    /// The sending device's signature key, base64url.
    pub device: String,
    pub channel: String,
    pub hlc: String,
    pub author_seq: u64,
    /// The latest text (Markdown); the last text before a deletion.
    pub content: String,
    /// Every version, the first included, in log order.
    pub versions: Vec<Version>,
    pub deleted: bool,
    /// Sent from this device.
    #[serde(default)]
    pub own: bool,
    /// The id of the note it answers.
    #[serde(default)]
    pub in_reply_to: Option<String>,
    /// Files it carries.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

impl Message {
    pub fn edited(&self) -> bool {
        self.versions.len() > 1
    }
}

/// Who sent an activity, and from where.
#[derive(Debug, Clone, Copy)]
pub struct From<'a> {
    pub seq: u64,
    pub sender: &'a str,
    pub device: &'a str,
    pub own: bool,
}

fn get<T: for<'de> Deserialize<'de>>(p: &Provider<'_>, k: &[u8]) -> Result<Option<T>, CoreError> {
    p.get(Table::Messages, k)?
        .map(|v| serde_json::from_slice(&v).map_err(|_| err("a stored message is damaged")))
        .transpose()
}

fn put<T: Serialize>(p: &Provider<'_>, k: &[u8], v: &T) -> Result<(), CoreError> {
    let bytes = Zeroizing::new(serde_json::to_vec(v).map_err(|_| err("encoding"))?);
    p.set(Table::Messages, k, Some(bytes.to_vec()))
}

/// Records one activity within the caller's batch. Activities this client
/// doesn't show yet are only counted.
pub fn record(p: &Provider<'_>, gid: &[u8], from: From<'_>, a: &Activity) -> Result<(), CoreError> {
    if let Some(n) = a.author_seq {
        let k = key(gid, SEEN, from.device.as_bytes());
        let mut seen: BTreeSet<u64> = get(p, &k)?.unwrap_or_default();
        seen.insert(n);
        put(p, &k, &seen)?;
    }
    // A malformed name can't make a channel: the note goes to `story`.
    let channel = a
        .channel
        .as_deref()
        .and_then(|c| validate::channel(c).ok())
        .unwrap_or(DEFAULT_CHANNELS[0])
        .to_owned();
    if let Some(c) = a.channel_object() {
        return describe(p, gid, from, a, c);
    }
    match a.kind.as_str() {
        "Create" => {
            let Some(note) = a.note() else {
                return Ok(());
            };
            name_channel(p, gid, &channel)?;
            let index = key(gid, INDEX, note.id.as_bytes());
            if p.get(Table::Messages, &index)?.is_some() {
                // A note id seen before: the first one stands.
                return Ok(());
            }
            let hlc = a.clock().to_string();
            put(
                p,
                &key(gid, NOTE, &from.seq.to_be_bytes()),
                &Message {
                    seq: from.seq,
                    id: note.id.clone(),
                    sender: from.sender.to_owned(),
                    device: from.device.to_owned(),
                    channel,
                    hlc: hlc.clone(),
                    author_seq: a.author_seq.unwrap_or(0),
                    content: note.content.clone(),
                    versions: vec![Version {
                        seq: from.seq,
                        hlc,
                        content: Some(note.content.clone()),
                    }],
                    deleted: false,
                    own: from.own,
                    in_reply_to: note.in_reply_to.clone(),
                    attachments: note.attachment.clone(),
                },
            )?;
            // Others' files are fetched before the message is acknowledged;
            // this device's own are kept as they're sent.
            if !from.own {
                for a in &note.attachment {
                    put(p, &key(gid, PENDING, a.url.as_bytes()), a)?;
                }
            }
            put(p, &index, &from.seq)
        }
        "Update" | "Delete" => {
            let Some(at) = get::<u64>(p, &key(gid, INDEX, a.target().as_bytes()))? else {
                return Ok(());
            };
            let k = key(gid, NOTE, &at.to_be_bytes());
            let Some(mut m) = get::<Message>(p, &k)? else {
                return Ok(());
            };
            // Only the note's own author can change it, and a deleted note
            // stays deleted.
            if m.sender != from.sender || m.deleted {
                return Ok(());
            }
            let content = if a.kind == "Update" {
                match a.note() {
                    Some(n) => Some(n.content.clone()),
                    None => return Ok(()),
                }
            } else {
                None
            };
            m.versions.push(Version {
                seq: from.seq,
                hlc: a.clock().to_string(),
                content: content.clone(),
            });
            match content {
                Some(text) => m.content = text,
                None => m.deleted = true,
            }
            put(p, &k, &m)
        }
        _ => Ok(()),
    }
}

/// A channel as this device knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInfo {
    pub name: String,
    /// Its current description.
    #[serde(default)]
    pub summary: Option<String>,
    /// Every description, in log order.
    #[serde(default)]
    pub versions: Vec<Version>,
    /// Notes in it, and how many this device hasn't shown yet (filled in
    /// by [`channels`]).
    #[serde(default)]
    pub notes: usize,
    #[serde(default)]
    pub unseen: usize,
}

/// Notes that a channel exists (writing in it is enough).
fn name_channel(p: &Provider<'_>, gid: &[u8], name: &str) -> Result<(), CoreError> {
    let k = key(gid, CHANNEL, name.as_bytes());
    if p.get(Table::Messages, &k)?.is_none() {
        put(
            p,
            &k,
            &ChannelInfo {
                name: name.to_owned(),
                summary: None,
                versions: Vec::new(),
                notes: 0,
                unseen: 0,
            },
        )?;
    }
    Ok(())
}

/// A channel named or described. Either writer may describe any channel;
/// every description is kept.
fn describe(
    p: &Provider<'_>,
    gid: &[u8],
    from: From<'_>,
    a: &Activity,
    c: &crate::activity::Channel,
) -> Result<(), CoreError> {
    if !matches!(a.kind.as_str(), "Create" | "Update") || validate::channel(&c.name).is_err() {
        return Ok(());
    }
    name_channel(p, gid, &c.name)?;
    let k = key(gid, CHANNEL, c.name.as_bytes());
    let mut info: ChannelInfo = get(p, &k)?.unwrap_or_else(|| ChannelInfo {
        name: c.name.clone(),
        summary: None,
        versions: Vec::new(),
        notes: 0,
        unseen: 0,
    });
    info.summary.clone_from(&c.summary);
    info.versions.push(Version {
        seq: from.seq,
        hlc: a.clock().to_string(),
        content: c.summary.clone(),
    });
    put(p, &k, &info)
}

/// A session's channels: `story` and `planning`, then the rest by name,
/// with how many notes each has and how many this device hasn't shown.
pub fn channels(store: &Store, gid: &[u8]) -> Result<Vec<ChannelInfo>, CoreError> {
    let p = Provider::new(store);
    let mut out: Vec<ChannelInfo> = Vec::new();
    for name in DEFAULT_CHANNELS {
        out.push(
            get(&p, &key(gid, CHANNEL, name.as_bytes()))?.unwrap_or_else(|| ChannelInfo {
                name: name.to_owned(),
                summary: None,
                versions: Vec::new(),
                notes: 0,
                unseen: 0,
            }),
        );
    }
    for (_, v) in store.scan(Table::Messages, &prefix(gid, CHANNEL))? {
        let info: ChannelInfo =
            serde_json::from_slice(&v).map_err(|_| err("a stored channel is damaged"))?;
        if !DEFAULT_CHANNELS.contains(&info.name.as_str()) {
            out.push(info);
        }
    }
    let notes = messages(store, gid)?;
    for c in &mut out {
        let shown: u64 = get(&p, &key(gid, SHOWN, c.name.as_bytes()))?.unwrap_or(0);
        let here = notes.iter().filter(|m| m.channel == c.name);
        c.notes = here.clone().count();
        c.unseen = here.filter(|m| !m.own && m.seq > shown).count();
    }
    Ok(out)
}

/// Notes that this device has shown `channel`'s notes up to `seq`.
pub fn mark_shown(store: &Store, gid: &[u8], channel: &str, seq: u64) -> Result<(), CoreError> {
    let p = Provider::new(store);
    p.atomically(|p| {
        let k = key(gid, SHOWN, channel.as_bytes());
        let shown: u64 = get(p, &k)?.unwrap_or(0);
        if seq > shown {
            put(p, &k, &seq)
        } else {
            Ok(())
        }
    })
}

/// Where the note `id` is: its position and channel, if this device has it.
pub fn find_note(store: &Store, gid: &[u8], id: &str) -> Result<Option<(u64, String)>, CoreError> {
    let p = Provider::new(store);
    let Some(seq) = get::<u64>(&p, &key(gid, INDEX, id.as_bytes()))? else {
        return Ok(None);
    };
    Ok(message(store, gid, seq)?.map(|m| (seq, m.channel)))
}

/// Attachments still to fetch.
pub fn pending(store: &Store, gid: &[u8]) -> Result<Vec<Attachment>, CoreError> {
    store
        .scan(Table::Messages, &prefix(gid, PENDING))?
        .into_iter()
        .map(|(_, v)| serde_json::from_slice(&v).map_err(|_| err("a stored attachment is damaged")))
        .collect()
}

/// Keeps an attachment's contents (or notes it can't be had, with `None`),
/// and stops waiting for it.
pub fn keep_file(
    store: &Store,
    gid: &[u8],
    url: &str,
    contents: Option<&[u8]>,
) -> Result<(), CoreError> {
    Provider::new(store).atomically(|p| keep_file_in(p, gid, url, contents))
}

/// [`keep_file`] within the caller's batch.
pub fn keep_file_in(
    p: &Provider<'_>,
    gid: &[u8],
    url: &str,
    contents: Option<&[u8]>,
) -> Result<(), CoreError> {
    if let Some(c) = contents {
        p.set(Table::Files, url.as_bytes(), Some(c.to_vec()))?;
    }
    p.set(Table::Messages, &key(gid, PENDING, url.as_bytes()), None)
}

/// An attachment's contents, if this device has them.
pub fn file(store: &Store, url: &str) -> Result<Option<Zeroizing<Vec<u8>>>, CoreError> {
    Ok(store
        .get(Table::Files, url.as_bytes())?
        .map(|v| Zeroizing::new(v.to_vec())))
}

/// The note created at `seq`.
pub fn message(store: &Store, gid: &[u8], seq: u64) -> Result<Option<Message>, CoreError> {
    get(&Provider::new(store), &key(gid, NOTE, &seq.to_be_bytes()))
}

/// A session's notes in display order: by clock, then sender, then
/// `authorSeq`.
pub fn messages(store: &Store, gid: &[u8]) -> Result<Vec<Message>, CoreError> {
    let mut out: Vec<Message> = store
        .scan(Table::Messages, &prefix(gid, NOTE))?
        .into_iter()
        .map(|(_, v)| serde_json::from_slice(&v).map_err(|_| err("a stored message is damaged")))
        .collect::<Result<_, _>>()?;
    out.sort_by(|a, b| {
        display_key(&a.hlc, &a.sender, a.author_seq).cmp(&display_key(
            &b.hlc,
            &b.sender,
            b.author_seq,
        ))
    });
    Ok(out)
}

/// What display order sorts by: the clock (a malformed value reads as
/// zero), then the sender's actor URI by its bytes, then `authorSeq`.
pub fn display_key<'a>(hlc: &str, sender: &'a str, author_seq: u64) -> (Hlc, &'a [u8], u64) {
    (
        hlc.parse().unwrap_or_default(),
        sender.as_bytes(),
        author_seq,
    )
}

/// How many notes this device holds for a session.
pub fn count(store: &Store, gid: &[u8]) -> Result<usize, CoreError> {
    Ok(store.scan(Table::Messages, &prefix(gid, NOTE))?.len())
}

/// Numbers missing from a device's `authorSeq` values: activities the
/// server didn't deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    /// The device's signature key, base64url.
    pub device: String,
    pub missing: Vec<u64>,
}

pub fn gaps(store: &Store, gid: &[u8]) -> Result<Vec<Gap>, CoreError> {
    let start = prefix(gid, SEEN);
    let mut out = Vec::new();
    for (k, v) in store.scan(Table::Messages, &start)? {
        let seen: BTreeSet<u64> =
            serde_json::from_slice(&v).map_err(|_| err("a stored message is damaged"))?;
        let (Some(first), Some(last)) = (seen.first(), seen.last()) else {
            continue;
        };
        let missing: Vec<u64> = (*first..*last).filter(|n| !seen.contains(n)).collect();
        if !missing.is_empty() {
            out.push(Gap {
                device: String::from_utf8_lossy(&k[start.len()..]).into_owned(),
                missing,
            });
        }
    }
    Ok(out)
}

// ---- Sharing history with the login's other devices (2k) -------------------

/// Part of a session's history, as one device of the login sends it to
/// another: notes with every version, channels with every description, and
/// the `authorSeq` values seen. What this device has shown and what it
/// still has to fetch stay here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rows {
    #[serde(default)]
    pub notes: Vec<Message>,
    #[serde(default)]
    pub channels: Vec<ChannelInfo>,
    /// Per sending device (signature key, base64url).
    #[serde(default)]
    pub seen: Vec<(String, BTreeSet<u64>)>,
}

/// The notes with a version logged in `from..=to`, and every channel and
/// count, for another device of the login.
pub fn export(store: &Store, gid: &[u8], from: u64, to: u64) -> Result<Rows, CoreError> {
    let notes = messages(store, gid)?
        .into_iter()
        .filter(|m| m.versions.iter().any(|v| (from..=to).contains(&v.seq)))
        .map(|mut m| {
            m.own = false;
            m
        })
        .collect();
    let mut channels = Vec::new();
    for (_, v) in store.scan(Table::Messages, &prefix(gid, CHANNEL))? {
        let mut c: ChannelInfo =
            serde_json::from_slice(&v).map_err(|_| err("a stored channel is damaged"))?;
        c.notes = 0;
        c.unseen = 0;
        channels.push(c);
    }
    let start = prefix(gid, SEEN);
    let mut seen = Vec::new();
    for (k, v) in store.scan(Table::Messages, &start)? {
        let set: BTreeSet<u64> =
            serde_json::from_slice(&v).map_err(|_| err("a stored message is damaged"))?;
        seen.push((String::from_utf8_lossy(&k[start.len()..]).into_owned(), set));
    }
    Ok(Rows {
        notes,
        channels,
        seen,
    })
}

/// Joins two lists of versions by log position; the result is in log
/// order. Nothing is dropped.
fn join_versions(mine: &[Version], theirs: &[Version]) -> Vec<Version> {
    let mut all: Vec<Version> = mine.to_vec();
    for v in theirs {
        if !all.iter().any(|m| m.seq == v.seq) {
            all.push(v.clone());
        }
    }
    all.sort_by_key(|v| v.seq);
    all
}

/// Merges rows from another device of the login, within the caller's
/// batch: notes this device lacks are added, notes it has gain the
/// versions they lack, channels their descriptions, counts their values.
/// Attachments not here yet are noted to fetch.
pub fn merge(p: &Provider<'_>, gid: &[u8], rows: &Rows) -> Result<(), CoreError> {
    for m in &rows.notes {
        if m.versions.is_empty() {
            continue;
        }
        let k = key(gid, NOTE, &m.seq.to_be_bytes());
        let index = key(gid, INDEX, m.id.as_bytes());
        let merged = match get::<Message>(p, &k)? {
            Some(mut have) => {
                if have.id != m.id || have.sender != m.sender {
                    // Not the same note: this device's stands.
                    continue;
                }
                have.versions = join_versions(&have.versions, &m.versions);
                have
            }
            None => {
                if get::<u64>(p, &index)?.is_some_and(|at| at != m.seq) {
                    // A note id seen at another position: the first stands.
                    continue;
                }
                let mut fresh = m.clone();
                fresh.own = false;
                fresh.versions = join_versions(&[], &m.versions);
                for a in &fresh.attachments {
                    if p.get(Table::Files, a.url.as_bytes())?.is_none() {
                        put(p, &key(gid, PENDING, a.url.as_bytes()), a)?;
                    }
                }
                fresh
            }
        };
        let mut merged = merged;
        // The current state follows from the versions: the last text, and
        // deleted if the last version deleted it.
        if let Some(text) = merged.versions.iter().rev().find_map(|v| v.content.clone()) {
            merged.content = text;
        }
        merged.deleted = merged.versions.last().is_some_and(|v| v.content.is_none());
        name_channel(p, gid, &merged.channel)?;
        put(p, &k, &merged)?;
        put(p, &index, &merged.seq)?;
    }
    for c in &rows.channels {
        if validate::channel(&c.name).is_err() {
            continue;
        }
        let k = key(gid, CHANNEL, c.name.as_bytes());
        let mut info: ChannelInfo = get(p, &k)?.unwrap_or_else(|| ChannelInfo {
            name: c.name.clone(),
            summary: None,
            versions: Vec::new(),
            notes: 0,
            unseen: 0,
        });
        info.versions = join_versions(&info.versions, &c.versions);
        info.summary = info.versions.last().and_then(|v| v.content.clone());
        put(p, &k, &info)?;
    }
    for (device, values) in &rows.seen {
        let k = key(gid, SEEN, device.as_bytes());
        let mut seen: BTreeSet<u64> = get(p, &k)?.unwrap_or_default();
        seen.extend(values.iter().copied());
        put(p, &k, &seen)?;
    }
    Ok(())
}

/// The highest log position this device holds a note or version from.
pub fn latest(store: &Store, gid: &[u8]) -> Result<u64, CoreError> {
    let mut top = 0;
    for m in messages(store, gid)? {
        top = top.max(m.versions.iter().map(|v| v.seq).max().unwrap_or(m.seq));
    }
    for c in channels(store, gid)? {
        top = top.max(c.versions.iter().map(|v| v.seq).max().unwrap_or(0));
    }
    Ok(top)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GID: &[u8] = &[7u8; 32];

    fn h(ms: u64) -> Hlc {
        Hlc { ms, counter: 0 }
    }

    fn from<'a>(seq: u64, sender: &'a str, device: &'a str) -> From<'a> {
        From {
            seq,
            sender,
            device,
            own: false,
        }
    }

    fn rec(store: &Store, f: From<'_>, a: &Activity) {
        Provider::new(store)
            .atomically(|p| record(p, GID, f, a))
            .unwrap();
    }

    #[test]
    fn notes_sort_by_clock_and_keep_every_version() {
        let store = Store::in_memory();
        let first = Activity::create("Second by clock", "story", h(200), 1);
        let second = Activity::create("First by clock", "story", h(100), 1);
        rec(&store, from(1, "alice", "A"), &first);
        rec(&store, from(2, "bob", "B"), &second);
        let shown: Vec<String> = messages(&store, GID)
            .unwrap()
            .into_iter()
            .map(|m| m.content)
            .collect();
        assert_eq!(shown, ["First by clock", "Second by clock"]);

        // Alice edits hers; Bob's attempt to edit or delete it is ignored.
        rec(
            &store,
            from(3, "alice", "A"),
            &Activity::update(first.target(), "Edited", "story", h(300), 2),
        );
        rec(
            &store,
            from(4, "bob", "B"),
            &Activity::update(first.target(), "Bob's words", "story", h(400), 2),
        );
        rec(
            &store,
            from(5, "bob", "B"),
            &Activity::delete(first.target(), "story", h(500), 3),
        );
        let m = message(&store, GID, 1).unwrap().unwrap();
        assert_eq!(m.content, "Edited");
        assert!(m.edited() && !m.deleted);
        assert_eq!(m.versions.len(), 2);

        // Alice deletes it: hidden, history kept, and it stays deleted.
        rec(
            &store,
            from(6, "alice", "A"),
            &Activity::delete(first.target(), "story", h(600), 3),
        );
        rec(
            &store,
            from(7, "alice", "A"),
            &Activity::update(first.target(), "Back?", "story", h(700), 4),
        );
        let m = message(&store, GID, 1).unwrap().unwrap();
        assert!(m.deleted);
        assert_eq!(m.content, "Edited");
        assert_eq!(
            m.versions
                .iter()
                .map(|v| v.content.clone())
                .collect::<Vec<_>>(),
            [Some("Second by clock".into()), Some("Edited".into()), None]
        );
        // A repeated note id doesn't replace the first.
        let mut copy = Activity::create("Imposter", "story", h(800), 5);
        copy.object = first.object.clone();
        rec(&store, from(8, "alice", "A"), &copy);
        assert_eq!(count(&store, GID).unwrap(), 2);
    }

    #[test]
    fn channels_are_named_described_and_counted() {
        let store = Store::in_memory();
        let names = |store: &Store| -> Vec<String> {
            channels(store, GID)
                .unwrap()
                .into_iter()
                .map(|c| c.name)
                .collect()
        };
        assert_eq!(names(&store), ["story", "planning"]);
        rec(
            &store,
            from(1, "alice", "A"),
            &Activity::channel("Create", "world", Some("Places"), h(1), 1),
        );
        rec(
            &store,
            from(2, "bob", "B"),
            &Activity::channel("Update", "world", Some("Places and people"), h(2), 1),
        );
        let note = Activity::create("The lighthouse.", "world", h(3), 3);
        rec(&store, from(3, "bob", "B"), &note);
        // Writing in a channel names it; a malformed name files under story.
        rec(
            &store,
            from(4, "bob", "B"),
            &Activity::create("Act two.", "act-2", h(4), 4).replying_to(note.target()),
        );
        rec(
            &store,
            from(5, "bob", "B"),
            &Activity::create("Bad name.", "Not A Name", h(5), 5),
        );
        rec(
            &store,
            from(6, "bob", "B"),
            &Activity::channel("Create", "no good", Some("x"), h(6), 6),
        );
        assert_eq!(names(&store), ["story", "planning", "act-2", "world"]);
        let all = channels(&store, GID).unwrap();
        let world = all.iter().find(|c| c.name == "world").unwrap();
        assert_eq!(world.summary.as_deref(), Some("Places and people"));
        assert_eq!(world.versions.len(), 2);
        assert_eq!((world.notes, world.unseen), (1, 1));
        assert_eq!(all[0].notes, 1, "the badly named note is in story");

        // The reply points across channels.
        let reply = message(&store, GID, 4).unwrap().unwrap();
        let target = reply.in_reply_to.unwrap();
        assert_eq!(
            find_note(&store, GID, &target).unwrap(),
            Some((3, "world".into()))
        );

        mark_shown(&store, GID, "world", 3).unwrap();
        let world = channels(&store, GID)
            .unwrap()
            .into_iter()
            .find(|c| c.name == "world")
            .unwrap();
        assert_eq!(world.unseen, 0);
    }

    #[test]
    fn missing_author_numbers_show_as_gaps() {
        let store = Store::in_memory();
        for (seq, n) in [(1, 1), (2, 2), (3, 5), (4, 6)] {
            rec(
                &store,
                from(seq, "alice", "A"),
                &Activity::create("x", "story", h(seq), n),
            );
        }
        // A device seen first part-way through has no gap before it.
        for (seq, n) in [(5, 40), (6, 41)] {
            rec(
                &store,
                from(seq, "bob", "B"),
                &Activity::create("y", "story", h(seq), n),
            );
        }
        assert_eq!(
            gaps(&store, GID).unwrap(),
            [Gap {
                device: "A".into(),
                missing: vec![3, 4]
            }]
        );
    }

    #[test]
    fn history_from_another_device_merges_without_losing_a_version() {
        // The device that was there throughout.
        let full = Store::in_memory();
        // One that joined at position 4 and read on from there.
        let late = Store::in_memory();
        let note = Activity::create("Opening.", "story", h(1), 1);
        let world = Activity::channel("Create", "world", Some("Places"), h(2), 2);
        let second = Activity::create("Second.", "world", h(3), 3);
        let edit = Activity::update(note.target(), "Opening, edited.", "story", h(5), 4);
        let gone = Activity::delete(second.target(), "world", h(6), 5);
        let third = Activity::create("Third.", "story", h(7), 6);
        let script: [(u64, &Activity); 6] = [
            (1, &note),
            (2, &world),
            (3, &second),
            (5, &edit),
            (6, &gone),
            (7, &third),
        ];
        for (seq, a) in script {
            rec(&full, from(seq, "alice", "A"), a);
            if seq >= 5 {
                rec(&late, from(seq, "alice", "A"), a);
            }
        }
        // Updates for notes it never saw were dropped; it holds only
        // "Third." so far.
        assert_eq!(count(&late, GID).unwrap(), 1);
        assert_eq!(latest(&late, GID).unwrap(), 7);

        let rows = export(&full, GID, 1, 4).unwrap();
        assert_eq!(rows.notes.len(), 2, "both notes created before 4");
        // Merging twice changes nothing more.
        for _ in 0..2 {
            Provider::new(&late)
                .atomically(|p| merge(p, GID, &rows))
                .unwrap();
        }
        assert_eq!(messages(&late, GID).unwrap(), messages(&full, GID).unwrap());
        assert_eq!(channels(&late, GID).unwrap(), channels(&full, GID).unwrap());
        assert_eq!(gaps(&late, GID).unwrap(), gaps(&full, GID).unwrap());

        // A version only the receiver has is kept too.
        let mut theirs = export(&full, GID, 1, 7).unwrap();
        for m in &mut theirs.notes {
            m.versions.retain(|v| v.seq < 5);
        }
        Provider::new(&late)
            .atomically(|p| merge(p, GID, &theirs))
            .unwrap();
        let opening = message(&late, GID, 1).unwrap().unwrap();
        assert_eq!(opening.content, "Opening, edited.");
        assert_eq!(opening.versions.len(), 2);
        assert!(message(&late, GID, 3).unwrap().unwrap().deleted);
    }
}
