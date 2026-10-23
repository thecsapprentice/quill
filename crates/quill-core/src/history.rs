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
//! - `0x02` + device key: the `authorSeq` values seen from that device.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{CoreError, Store, activity::Activity, hlc::Hlc, mls::Provider, store::kv::Table};

const NOTE: u8 = 0;
const INDEX: u8 = 1;
const SEEN: u8 = 2;

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
}

impl Message {
    pub fn edited(&self) -> bool {
        self.versions.len() > 1
    }

    fn clock(&self) -> Hlc {
        self.hlc.parse().unwrap_or_default()
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
    let channel = a.channel.clone().unwrap_or_else(|| "story".into());
    match a.kind.as_str() {
        "Create" => {
            let Some(note) = a.note() else {
                return Ok(());
            };
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
                },
            )?;
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
        (a.clock(), &a.sender, a.author_seq).cmp(&(b.clock(), &b.sender, b.author_seq))
    });
    Ok(out)
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
}
