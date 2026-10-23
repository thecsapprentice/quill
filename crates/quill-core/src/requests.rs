//! Collaboration requests (2e; protocol draft, section 4).
//!
//! The requester's device builds the session's MLS group: it claims a
//! KeyPackage for each of the prompt author's devices (and its own
//! persona's other devices), adds them all, and encrypts the pitch as the
//! group's first application message. The server files the Welcome and the
//! pitch without reading either. The author's device joins from the Welcome
//! to read the pitch, checking every member against the device directory
//! first, then accepts, declines or lets it lapse.
//!
//! Each device keeps a note (in [`Table::Requests`]) of the requests it holds
//! a group for, so upkeep can drop the groups of requests that went nowhere.

use std::collections::HashSet;

use openmls::prelude::{
    BasicCredential, Extension, Extensions, GroupId, KeyPackage, MlsGroup, MlsGroupCreateConfig,
    MlsMessageBodyIn, MlsMessageIn, MlsMessageOut, PURE_CIPHERTEXT_WIRE_FORMAT_POLICY,
    ProcessedMessageContent, Sender, StagedWelcome, UnknownExtension, WelcomeError,
};
use openmls_traits::{OpenMlsProvider as _, storage::StorageProvider as _};
use quill_proto::{
    api::Persona,
    keys::{actor_uri, b64, unb64},
    requests::{
        IncomingRequest, OutgoingRequest, RequestPersona, RequestSent, RequestStatus, SendRequest,
    },
};
use serde::{Deserialize, Serialize};
use tls_codec::{Deserialize as _, Serialize as _};
use uuid::Uuid;
use zeroize::Zeroizing;

pub use crate::activity::Activity;
use crate::{
    Client, CoreError, Store,
    devices::{self, PAST_EPOCHS},
    hlc::Hlc,
    keys::{self, DeviceStatus, PinStatus},
    mls::{self, CIPHERSUITE, DEVICE_CERT_EXTENSION, DeviceSigner, Provider, mls_err},
    store::kv::{Op, Table},
};

/// The longest pitch, in characters.
pub const MAX_PITCH_CHARS: usize = 4000;

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

// ---- The pitch -------------------------------------------------------------------

/// The pitch's text, if `plain` is a pitch.
pub(crate) fn pitch_of(plain: &[u8]) -> Result<Activity, CoreError> {
    let a: Activity = serde_json::from_slice(plain).map_err(|_| err("the pitch is unreadable"))?;
    let Some(note) = a.note().filter(|_| a.kind == "Create") else {
        return Err(err("the first message isn't a pitch"));
    };
    if note.content.chars().count() > MAX_PITCH_CHARS {
        return Err(err("the pitch is too long"));
    }
    Ok(a)
}

/// The pitch as a device keeps it until the request becomes a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pitch {
    pub(crate) activity: Activity,
    /// The sending device's signature key, base64url.
    pub(crate) device: String,
}

// ---- What this device remembers ----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Role {
    Outgoing,
    Incoming,
}

/// A request this device holds a group for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Held {
    pub(crate) role: Role,
    pub(crate) group_id: String,
    /// This login's persona in it.
    pub(crate) persona: Uuid,
    #[serde(default)]
    pub(crate) requester: Option<String>,
    /// The pitch: as sent (outgoing), or as read (incoming), since MLS
    /// decrypts each message once and never one's own.
    #[serde(default)]
    pub(crate) pitch: Option<Pitch>,
    #[serde(default)]
    pub(crate) accepted: bool,
}

pub(crate) fn held(store: &Store, id: Uuid) -> Result<Option<Held>, CoreError> {
    store
        .get(Table::Requests, id.as_bytes())?
        .map(|v| serde_json::from_slice(&v).map_err(|_| err("a stored request is damaged")))
        .transpose()
}

fn encode_held(h: &Held) -> Result<Vec<u8>, CoreError> {
    serde_json::to_vec(h).map_err(|_| err("encoding"))
}

fn save_held(store: &Store, id: Uuid, h: &Held) -> Result<(), CoreError> {
    let v = Zeroizing::new(encode_held(h)?);
    Ok(store.apply(&[Op::Put {
        table: Table::Requests,
        key: id.as_bytes(),
        value: &v,
    }])?)
}

/// Forgets a request: its group's MLS state and the note, in one batch.
fn forget(store: &Store, id: Uuid, group_id: Option<&str>) -> Result<(), CoreError> {
    let provider = Provider::new(store);
    provider.atomically(|p| {
        if let Some(gid) = group_id.and_then(unb64)
            && let Some(mut g) = MlsGroup::load(p.storage(), &GroupId::from_slice(&gid))
                .map_err(|e| mls_err("loading a request's group", e))?
        {
            g.delete(p.storage())
                .map_err(|e| mls_err("forgetting a request's group", e))?;
        }
        p.set(Table::Requests, id.as_bytes(), None)
    })
}

fn encode(m: &MlsMessageOut) -> Result<Vec<u8>, CoreError> {
    m.tls_serialize_detached()
        .map_err(|e| mls_err("encoding", e))
}

// ---- Sending ---------------------------------------------------------------------------

/// What [`send`] did.
#[derive(Debug, Clone)]
pub struct Sent {
    pub request: RequestSent,
    /// The author's devices the request reached.
    pub author_devices: usize,
    /// This persona's other devices put in the group.
    pub own_devices: usize,
    /// Devices left out, with why (their KeyPackages failed the checks).
    pub skipped: Vec<(Uuid, String)>,
}

pub(crate) fn session_config(device: &keys::DeviceKey) -> Result<MlsGroupCreateConfig, CoreError> {
    let extensions = Extensions::single(Extension::Unknown(
        DEVICE_CERT_EXTENSION,
        UnknownExtension(device.certificate.clone()),
    ))
    .map_err(|e| mls_err("leaf extension", e))?;
    MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .max_past_epochs(PAST_EPOCHS)
        .padding_size(devices::PADDING)
        .capabilities(mls::capabilities())
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .with_leaf_node_extensions(extensions)
        .map_err(|e| mls_err("leaf extension", e))
        .map(|b| b.build())
}

/// Sends a request to join `prompt` as `persona`, with `pitch`.
pub async fn send(
    store: &Store,
    client: &Client,
    persona: &Persona,
    prompt: Uuid,
    pitch: &str,
    expires_in_days: Option<u32>,
) -> Result<Sent, CoreError> {
    let pitch = pitch.trim();
    if pitch.is_empty() {
        return Err(err("say something in the pitch"));
    }
    if pitch.chars().count() > MAX_PITCH_CHARS {
        return Err(err(format!(
            "the pitch is too long (at most {MAX_PITCH_CHARS} characters)"
        )));
    }
    let device = keys::load_device(store, persona.id)?;
    let (device, my_device) = match device {
        Some(d) => match d.id {
            Some(id) => (d, id),
            None => return Err(not_set_up(&persona.handle)),
        },
        None => return Err(not_set_up(&persona.handle)),
    };
    let target = client.prompt(prompt).await?;
    let domain = client.instance().await?.domain;
    let author_uri = actor_uri(&domain, &target.author.handle);
    let my_uri = actor_uri(&domain, &persona.handle);

    let author = keys::claim(store, client, &target.author.handle, &author_uri).await?;
    if author.packages.is_empty() {
        return Err(err(format!(
            "@{} has no devices that can take a request right now",
            target.author.handle
        )));
    }
    let mut skipped = author.refused.clone();
    // This persona's other devices join from the start, if it has any.
    let mine = keys::check_published(
        &client.actor_keys(&persona.handle).await?,
        &my_uri,
        keys::now_ms(),
    )?;
    let others = mine
        .devices
        .iter()
        .any(|d| d.id != my_device && matches!(d.status, DeviceStatus::Valid { .. }));
    let mut own = Vec::new();
    if others {
        let claim = keys::claim(store, client, &persona.handle, &my_uri).await?;
        skipped.extend(claim.refused);
        for p in claim.packages {
            if p.device == my_device {
                // Claimed from this device's own pool: drop its private key.
                if !p.last_resort {
                    forget_own_package(store, &p.key_package)?;
                }
            } else {
                own.push(p.key_package);
            }
        }
    }
    let members: Vec<KeyPackage> = author
        .packages
        .iter()
        .map(|p| p.key_package.clone())
        .chain(own.iter().cloned())
        .collect();

    let mut gid = [0u8; 32];
    getrandom::fill(&mut gid).map_err(|e| err(format!("no randomness: {e}")))?;
    // The pitch is this device's first activity in the session.
    let activity = Activity::create(
        pitch,
        "planning",
        Hlc::send(Hlc::default(), keys::now_ms()),
        1,
    );
    let plain = Zeroizing::new(serde_json::to_vec(&activity).map_err(|_| err("encoding"))?);
    let config = session_config(&device)?;
    let provider = Provider::new(store);
    let (welcome, message) = provider.atomically(|p| {
        let signer = DeviceSigner(device.signing_key());
        let mut group = MlsGroup::new_with_group_id(
            p,
            &signer,
            &config,
            GroupId::from_slice(&gid),
            mls::credential(&my_uri, &device),
        )
        .map_err(|e| mls_err("creating the group", e))?;
        let (_commit, welcome, _) = group
            .add_members(p, &signer, &members)
            .map_err(|e| mls_err("adding members", e))?;
        // Nobody needs the commit: every member joins from the Welcome.
        group
            .merge_pending_commit(p)
            .map_err(|e| mls_err("merging", e))?;
        let message = group
            .create_message(p, &signer, &plain)
            .map_err(|e| mls_err("encrypting the pitch", e))?;
        Ok((encode(&welcome)?, encode(&message)?))
    })?;
    let group_id = b64(&gid);
    let sent = client
        .send_request(&SendRequest {
            persona: persona.id,
            prompt,
            welcome: b64(&welcome),
            pitch: b64(&message),
            expires_in_days,
        })
        .await;
    let request = match sent {
        Ok(r) => r,
        Err(e) => {
            forget(store, Uuid::nil(), Some(&group_id))?;
            return Err(e);
        }
    };
    save_held(
        store,
        request.id,
        &Held {
            role: Role::Outgoing,
            group_id,
            persona: persona.id,
            requester: None,
            pitch: Some(Pitch {
                activity,
                device: b64(&device.public()),
            }),
            accepted: false,
        },
    )?;
    Ok(Sent {
        request,
        author_devices: author.packages.len(),
        own_devices: own.len(),
        skipped,
    })
}

pub(crate) fn not_set_up(handle: &str) -> CoreError {
    err(format!(
        "@{handle} has no keys on this device: run `quill keys init --as {handle}` first"
    ))
}

/// Deletes the private key of one of this device's own KeyPackages.
fn forget_own_package(store: &Store, package: &KeyPackage) -> Result<(), CoreError> {
    let provider = Provider::new(store);
    let hash = package
        .hash_ref(provider.crypto())
        .map_err(|e| mls_err("hashing a KeyPackage", e))?;
    provider.atomically(|p| {
        p.storage()
            .delete_key_package(&hash)
            .map_err(|e| mls_err("forgetting a KeyPackage", e))
    })
}

// ---- The requester's side ----------------------------------------------------------------

/// The login's requests, as the server reports them.
pub async fn list(client: &Client) -> Result<Vec<OutgoingRequest>, CoreError> {
    client.own_requests().await
}

/// Withdraws (or dismisses) one of the login's requests.
pub async fn withdraw(store: &Store, client: &Client, id: Uuid) -> Result<(), CoreError> {
    client.withdraw_request(id).await?;
    let group = held(store, id)?.map(|h| h.group_id);
    forget(store, id, group.as_deref())
}

// ---- The author's side ---------------------------------------------------------------------

/// A request in the author's queue, as this device reads it.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub id: Uuid,
    pub requester: RequestPersona,
    pub received_at: time::OffsetDateTime,
    pub expires_at: time::OffsetDateTime,
    /// The pitch, or why this device can't show it.
    pub pitch: Result<String, String>,
}

/// A prompt's queue.
#[derive(Debug, Clone)]
pub struct Inbox {
    pub requests: Vec<Incoming>,
    /// Held requests not yet released (`held` releases them).
    pub held: u32,
}

/// Valid device keys of a persona, after checking its published keys.
pub(crate) type ValidKeys = HashSet<Vec<u8>>;

pub(crate) fn valid_keys(checked: &keys::CheckedKeys) -> ValidKeys {
    checked
        .devices
        .iter()
        .filter(|d| matches!(d.status, DeviceStatus::Valid { .. }))
        .map(|d| d.device_key.clone())
        .collect()
}

/// Reads a prompt's queue, joining each new request's group to read its
/// pitch. `held` releases held requests first.
pub async fn inbox(
    store: &Store,
    client: &Client,
    prompt: Uuid,
    held_too: bool,
) -> Result<Inbox, CoreError> {
    let target = client.prompt(prompt).await?;
    let personas = client.personas().await?;
    let me = personas
        .iter()
        .find(|p| p.id == target.author.id)
        .ok_or_else(|| err("that prompt isn't yours"))?;
    let queue = client.request_queue(prompt, held_too).await?;
    let domain = client.instance().await?.domain;
    let author_uri = actor_uri(&domain, &me.handle);
    let mut author_keys: Option<ValidKeys> = None;
    let mut out = Vec::new();
    for r in queue.requests {
        let pitch = if let Some(h) = held(store, r.id)? {
            let incoming = h.role == Role::Incoming;
            h.pitch
                .filter(|_| incoming)
                .and_then(|p| p.activity.note().map(|n| n.content.clone()))
                .ok_or_else(|| "this device sent it".to_owned())
        } else {
            if author_keys.is_none() {
                author_keys = Some(own_keys(store, client, me, &author_uri).await?);
            }
            let author_keys = author_keys.as_ref().unwrap_or_else(|| unreachable!());
            read(store, client, me, &domain, &author_uri, author_keys, &r)
                .await
                .map_err(|e| e.to_string())
        };
        out.push(Incoming {
            id: r.id,
            requester: r.requester,
            received_at: r.received_at,
            expires_at: r.expires_at,
            pitch,
        });
    }
    Ok(Inbox {
        requests: out,
        held: queue.held,
    })
}

/// The author persona's valid device keys, after checking its published
/// identity key is the one this device holds.
async fn own_keys(
    store: &Store,
    client: &Client,
    me: &Persona,
    uri: &str,
) -> Result<ValidKeys, CoreError> {
    let identity = keys::load_identity(store, me.id)?.ok_or_else(|| not_set_up(&me.handle))?;
    let checked =
        keys::check_published(&client.actor_keys(&me.handle).await?, uri, keys::now_ms())?;
    if checked.identity.as_ref().map(|(k, _)| k.as_slice()) != Some(identity.public().as_slice()) {
        return Err(err(format!(
            "the server publishes another identity key for @{}",
            me.handle
        )));
    }
    Ok(valid_keys(&checked))
}

/// Joins one request's group and reads its pitch, refusing a group with a
/// member that isn't a valid device of the requester or of the author, or
/// a pitch from anyone but the requester.
async fn read(
    store: &Store,
    client: &Client,
    me: &Persona,
    domain: &str,
    author_uri: &str,
    author_keys: &ValidKeys,
    r: &IncomingRequest,
) -> Result<String, CoreError> {
    let requester_uri = actor_uri(domain, &r.requester.handle);
    if r.requester.actor_uri != requester_uri {
        return Err(err("the server misnames the requester"));
    }
    let published = keys::check_published(
        &client.actor_keys(&r.requester.handle).await?,
        &requester_uri,
        keys::now_ms(),
    )?;
    let (key, version) = published
        .identity
        .clone()
        .ok_or_else(|| err("the requester has no identity key"))?;
    if let PinStatus::Changed { .. } = keys::check_pin(store, &requester_uri, &key, version)? {
        return Err(err(format!(
            "@{}'s identity key changed since it was pinned; check it with `quill keys show {}`",
            r.requester.handle, r.requester.handle
        )));
    }
    let requester_keys = valid_keys(&published);
    let allowed = |credential: &openmls::prelude::Credential, key: &[u8]| -> Option<bool> {
        let basic = BasicCredential::try_from(credential.clone()).ok()?;
        let who = basic.identity();
        if who == requester_uri.as_bytes() && requester_keys.contains(key) {
            Some(true)
        } else if who == author_uri.as_bytes() && author_keys.contains(key) {
            Some(false)
        } else {
            None
        }
    };

    let welcome = unb64(&r.welcome).ok_or_else(|| err("server sent bad base64"))?;
    let pitch = unb64(&r.pitch).ok_or_else(|| err("server sent bad base64"))?;
    let gid = unb64(&r.group_id).ok_or_else(|| err("server sent bad base64"))?;
    let welcome = match MlsMessageIn::tls_deserialize_exact(welcome)
        .map_err(|e| mls_err("decoding the Welcome", e))?
        .extract()
    {
        MlsMessageBodyIn::Welcome(w) => w,
        _ => return Err(err("the request holds no Welcome")),
    };
    let message = MlsMessageIn::tls_deserialize_exact(pitch)
        .map_err(|e| mls_err("decoding the pitch", e))?
        .try_into_protocol_message()
        .map_err(|e| mls_err("decoding the pitch", e))?;
    let provider = Provider::new(store);
    provider.atomically(|p| {
        let staged = match StagedWelcome::new_from_welcome(p, &devices::join_config(), welcome, None)
        {
            Ok(s) => s,
            Err(WelcomeError::NoMatchingKeyPackage) => {
                return Err(err(
                    "it wasn't sent to this device (linked after it was sent?)",
                ));
            }
            Err(e) => return Err(mls_err("joining the request's group", e)),
        };
        if staged.group_context().group_id().as_slice() != gid.as_slice() {
            return Err(err("the server filed it under another group"));
        }
        let mut requesters = 0;
        for m in staged.members() {
            match allowed(&m.credential, &m.signature_key) {
                Some(true) => requesters += 1,
                Some(false) => {}
                None => {
                    return Err(err(
                        "its group holds a device that is neither yours nor the requester's: refused",
                    ));
                }
            }
        }
        if requesters == 0 {
            return Err(err("its group holds none of the requester's devices"));
        }
        let mut group = staged
            .into_group(p)
            .map_err(|e| mls_err("joining the request's group", e))?;
        let processed = group
            .process_message(p, message)
            .map_err(|e| mls_err("decrypting the pitch", e))?;
        let Sender::Member(index) = *processed.sender() else {
            return Err(err("the pitch has no member sender"));
        };
        let sender_key = group
            .members()
            .find(|m| m.index == index)
            .map(|m| m.signature_key)
            .ok_or_else(|| err("the pitch's sender isn't in the group"))?;
        if allowed(processed.credential(), &sender_key) != Some(true) {
            return Err(err("the pitch doesn't come from the requester: refused"));
        }
        let ProcessedMessageContent::ApplicationMessage(app) = processed.into_content() else {
            return Err(err("the pitch isn't an application message"));
        };
        let plain = Zeroizing::new(app.into_bytes());
        let activity = pitch_of(&plain)?;
        let text = activity.note().map(|n| n.content.clone()).unwrap_or_default();
        let note = Held {
            role: Role::Incoming,
            group_id: r.group_id.clone(),
            persona: me.id,
            requester: Some(requester_uri.clone()),
            pitch: Some(Pitch {
                activity,
                device: b64(&sender_key),
            }),
            accepted: false,
        };
        p.set(Table::Requests, r.id.as_bytes(), Some(encode_held(&note)?))?;
        Ok(text)
    })
}

/// Accepts a request this device has read: the server makes its group a
/// session. Returns the session's group id.
pub async fn accept(store: &Store, client: &Client, id: Uuid) -> Result<String, CoreError> {
    let Some(mut note) = held(store, id)? else {
        return Err(err(
            "read the request on this device first (`quill request inbox PROMPT`)",
        ));
    };
    if note.role != Role::Incoming {
        return Err(err("that's a request this login sent"));
    }
    match client.accept_request(id).await {
        Ok(_) => {}
        Err(e) if e.status() == Some(409) => {
            // Accepted already, perhaps from another device.
            if client.request_status(id).await?.status != RequestStatus::Session {
                return Err(e);
            }
        }
        Err(e) => return Err(e),
    }
    note.accepted = true;
    save_held(store, id, &note)?;
    Ok(note.group_id)
}

/// Declines a request. The requester is told nothing.
pub async fn decline(store: &Store, client: &Client, id: Uuid) -> Result<(), CoreError> {
    client.decline_request(id).await?;
    let group = held(store, id)?.map(|h| h.group_id);
    forget(store, id, group.as_deref())
}

// ---- Upkeep --------------------------------------------------------------------------------

/// Drops the groups of requests that went nowhere (declined, withdrawn or
/// expired), and notes the ones that became sessions. Returns how many it
/// dropped.
pub async fn tidy(store: &Store, client: &Client) -> Result<usize, CoreError> {
    let mut dropped = 0;
    for (k, v) in store.scan(Table::Requests, b"")? {
        let (Ok(id), Ok(mut note)) = (Uuid::from_slice(&k), serde_json::from_slice::<Held>(&v))
        else {
            continue;
        };
        if note.accepted {
            continue;
        }
        let status = match client.request_status(id).await {
            Ok(s) => Some(s.status),
            Err(e) if e.status() == Some(404) => None,
            Err(e) => return Err(e),
        };
        match status {
            Some(RequestStatus::Session) => {
                note.accepted = true;
                save_held(store, id, &note)?;
            }
            Some(RequestStatus::Waiting | RequestStatus::Pending | RequestStatus::Held) => {}
            _ => {
                forget(store, id, Some(&note.group_id))?;
                dropped += 1;
            }
        }
    }
    Ok(dropped)
}

/// The group id of a request that became a session on this device, if any.
pub fn session_group(store: &Store, id: Uuid) -> Result<Option<String>, CoreError> {
    Ok(held(store, id)?.filter(|h| h.accepted).map(|h| h.group_id))
}
