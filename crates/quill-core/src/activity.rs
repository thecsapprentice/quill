//! What travels inside session messages (protocol draft, section 5): one
//! AS2 activity in UTF-8 JSON per MLS application message, with Quill's
//! fields for the clock (`hlc`), the sending device's counter
//! (`authorSeq`) and the channel.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::hlc::Hlc;

const CONTEXT: [&str; 2] = [
    "https://www.w3.org/ns/activitystreams",
    "https://quill.example/ns/wr/1.0",
];

/// One activity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    #[serde(rename = "@context")]
    pub context: Vec<String>,
    /// `Create`, `Update`, `Delete`, ...
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hlc: Option<String>,
    /// Counts this device's activities in the session, from 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_seq: Option<u64>,
    pub object: Object,
}

/// A `Note`, a channel, or (for `Delete`) the id of a note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Object {
    Note(Note),
    Channel(Channel),
    Id(String),
}

/// A named channel and what it's for (`wr:Channel`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// The type of a channel object.
pub const CHANNEL_TYPE: &str = "wr:Channel";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    pub content: String,
    /// The id of the note it answers, in any channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
}

fn urn() -> String {
    format!("urn:uuid:{}", Uuid::new_v4())
}

impl Activity {
    fn new(kind: &str, channel: Option<&str>, hlc: Hlc, author_seq: u64, object: Object) -> Self {
        Self {
            context: CONTEXT.iter().map(|s| (*s).to_owned()).collect(),
            kind: kind.into(),
            id: urn(),
            channel: channel.map(str::to_owned),
            hlc: Some(hlc.to_string()),
            author_seq: Some(author_seq),
            object,
        }
    }

    /// A new Markdown note in `channel`.
    pub fn create(text: &str, channel: &str, hlc: Hlc, author_seq: u64) -> Self {
        Self::new(
            "Create",
            Some(channel),
            hlc,
            author_seq,
            Object::Note(Note {
                kind: "Note".into(),
                id: urn(),
                media_type: Some("text/markdown".into()),
                content: text.to_owned(),
                in_reply_to: None,
            }),
        )
    }

    /// New text for the note `note_id`.
    pub fn update(note_id: &str, text: &str, channel: &str, hlc: Hlc, author_seq: u64) -> Self {
        Self::new(
            "Update",
            Some(channel),
            hlc,
            author_seq,
            Object::Note(Note {
                kind: "Note".into(),
                id: note_id.to_owned(),
                media_type: Some("text/markdown".into()),
                content: text.to_owned(),
                in_reply_to: None,
            }),
        )
    }

    /// Hides the note `note_id`.
    pub fn delete(note_id: &str, channel: &str, hlc: Hlc, author_seq: u64) -> Self {
        Self::new(
            "Delete",
            Some(channel),
            hlc,
            author_seq,
            Object::Id(note_id.to_owned()),
        )
    }

    /// Names a channel (`Create`), or describes it again (`Update`).
    pub fn channel(
        kind: &str,
        name: &str,
        summary: Option<&str>,
        hlc: Hlc,
        author_seq: u64,
    ) -> Self {
        Self::new(
            kind,
            Some(name),
            hlc,
            author_seq,
            Object::Channel(Channel {
                kind: CHANNEL_TYPE.into(),
                name: name.to_owned(),
                summary: summary.map(str::to_owned),
            }),
        )
    }

    /// The same note, answering the note `id`.
    pub fn replying_to(mut self, id: &str) -> Self {
        if let Object::Note(n) = &mut self.object {
            n.in_reply_to = Some(id.to_owned());
        }
        self
    }

    /// The channel object it carries, if any.
    pub fn channel_object(&self) -> Option<&Channel> {
        match &self.object {
            Object::Channel(c) if c.kind == CHANNEL_TYPE => Some(c),
            _ => None,
        }
    }

    /// The note it carries, if any.
    pub fn note(&self) -> Option<&Note> {
        match &self.object {
            Object::Note(n) if n.kind == "Note" => Some(n),
            _ => None,
        }
    }

    /// The id of the note it's about.
    pub fn target(&self) -> &str {
        match &self.object {
            Object::Note(n) => &n.id,
            Object::Channel(c) => &c.name,
            Object::Id(id) => id,
        }
    }

    /// Its clock value; an absent or malformed one reads as zero.
    pub fn clock(&self) -> Hlc {
        self.hlc
            .as_deref()
            .and_then(|h| h.parse().ok())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activities_round_trip_in_the_draft_shape() {
        let hlc: Hlc = "1760861400123-0004".parse().unwrap();
        let c = Activity::create("The ledger's last page.", "story", hlc, 42);
        let v: serde_json::Value = serde_json::to_value(&c).unwrap();
        assert_eq!(v["type"], "Create");
        assert_eq!(v["hlc"], "1760861400123-0004");
        assert_eq!(v["authorSeq"], 42);
        assert_eq!(v["channel"], "story");
        assert_eq!(v["object"]["type"], "Note");
        assert_eq!(v["@context"][0], "https://www.w3.org/ns/activitystreams");
        let back: Activity = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.clock(), hlc);

        let d = Activity::delete(c.target(), "story", hlc, 43);
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["object"], c.target());
        let back: Activity = serde_json::from_value(v).unwrap();
        assert_eq!(back.target(), c.target());
        assert!(back.note().is_none());

        let r = Activity::create("See planning.", "story", hlc, 44).replying_to(c.target());
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["object"]["inReplyTo"], c.target());
        let ch = Activity::channel("Create", "world", Some("Places"), hlc, 45);
        let v = serde_json::to_value(&ch).unwrap();
        assert_eq!(v["object"]["type"], "wr:Channel");
        let back: Activity = serde_json::from_value(v).unwrap();
        assert_eq!(
            back.channel_object().unwrap().summary.as_deref(),
            Some("Places")
        );
        assert!(back.note().is_none());
    }
}
