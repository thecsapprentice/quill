//! A login's devices (2d): the personal device group, linking a new
//! device, staying in sync, removing a device, and keeping KeyPackage
//! pools full.
//!
//! The personal device group is an MLS group holding only this login's
//! devices. Each device joins it with an **account device key**: never
//! published and tied to no persona, so the group says nothing about the
//! login's personas. Messages in it are application messages carrying a
//! JSON [`Payload`]; every one is encrypted and authenticated by MLS.
//!
//! Linking (protocol draft, section 6):
//! 1. the new device posts a KeyPackage to a 10-minute mailbox and shows a
//!    code containing the KeyPackage's SHA-256 fingerprint;
//! 2. a device already in use checks the KeyPackage against the code (so
//!    the server can't swap in its own), adds it to the group, sends it the
//!    personas' identity keys in the group, and answers with the Welcome;
//! 3. both show 6 words from the new epoch's authenticator, which match only
//!    if both are in the same group, so the server can't hand the new
//!    device a group of its own making. The new device takes the keys only
//!    after the user confirms.

use ed25519_dalek::SigningKey;
use openmls::prelude::{
    BasicCredential, Credential, CredentialWithKey, KeyPackage, LeafNodeIndex, Lifetime, MlsGroup,
    MlsGroupCreateConfig, MlsGroupJoinConfig, MlsMessageBodyIn, MlsMessageIn, MlsMessageOut,
    PURE_CIPHERTEXT_WIRE_FORMAT_POLICY, ProcessedMessageContent, ProtocolVersion, StagedWelcome,
};
use openmls_traits::OpenMlsProvider as _;
use quill_certs::RevocationReason;
use quill_proto::{
    api::Persona,
    groups::{CreateGroup, LinkReply, MessagesQuery, PostMessage, Posted, StartLink},
    keys::{b64, unb64},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tls_codec::{Deserialize as _, Serialize as _};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    Client, CoreError, Store, keys,
    mls::{self, CIPHERSUITE, DeviceSigner, Provider, mls_err},
    store::kv::{Op, Table},
    words,
};

/// The personal group's credentials: `quill-device:{account device id}`.
const CREDENTIAL_PREFIX: &str = "quill-device:";
const LINK_PREFIX: &str = "quill-link:1:";
/// Commits and messages are retried this often after losing an epoch race.
const RETRIES: usize = 5;

// Keys in `Table::Account`.
const K_DEVICE: &[u8] = b"device";
const K_GROUP: &[u8] = b"group";
const K_SEQ: &[u8] = b"seq";
const K_OWN: &[u8] = b"own";
const K_PENDING: &[u8] = b"pending-link";
const K_REMOVED: &[u8] = b"removed";
const K_ANNOUNCED: &[u8] = b"announced";
/// This device fell behind the device group: what it needed to follow it
/// was deleted, so it must be linked again (2k).
const K_STALE: &[u8] = b"stale";
/// History requests from the login's other devices, waiting for an answer.
pub(crate) const K_ASKS: &[u8] = b"history-asks";
/// History bundles for this device, waiting to be fetched and merged.
pub(crate) const K_INBOX: &[u8] = b"history-inbox";

fn err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

/// The error for a device that fell behind the device group.
fn fell_behind() -> CoreError {
    err(
        "this device was away longer than the server keeps your devices' messages: link it again with `quill device link` (its keys and sessions stay)",
    )
}

fn conflict(e: &CoreError) -> bool {
    e.status() == Some(409)
}

// ---- What travels in the personal group --------------------------------------

/// An application message in the personal device group.
#[derive(Debug, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Payload {
    /// Personas' identity keys and trust pins, for a newly linked device
    /// (protocol draft: `DeviceGrant`, `TrustPins`).
    DeviceGrant {
        identities: Vec<GrantedIdentity>,
        pins: Vec<GrantedPin>,
        /// What the sender knows of the login's devices (itself included),
        /// since their earlier announcements predate the new device.
        #[serde(default)]
        devices: Vec<DeviceAnnouncement>,
    },
    /// Which persona devices in the directory belong to the sender.
    DeviceKeys {
        device: Uuid,
        personas: Vec<PersonaDevice>,
    },
    /// The sender lacks a session's history from `from` to `to` (log
    /// positions, inclusive): any device that has it may answer (2k).
    HistoryRequest { session: String, from: u64, to: u64 },
    /// Part of a session's history for one device (protocol draft:
    /// `HistoryBundle`).
    HistoryBundle(HistoryBundle),
}

/// A session's history from one device of the login for another (2k): the
/// rows and files travel as encrypted blobs in the device group, their keys
/// here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryBundle {
    /// The account device it's for.
    pub for_device: Uuid,
    pub session: String,
    /// The log positions the sender vouches for (inclusive).
    pub from: u64,
    pub to: u64,
    pub parts: Vec<BundlePart>,
    /// The sender's last message for this request.
    pub last: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundlePart {
    /// For a file: the attachment URL the notes name it by. For rows, none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub blob: crate::attachments::Attachment,
}

/// A history request waiting on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Ask {
    /// The asking account device.
    pub device: Uuid,
    pub session: String,
    pub from: u64,
    pub to: u64,
    pub at_ms: u64,
}

pub(crate) fn load_list<T: for<'de> Deserialize<'de>>(
    p: &Provider<'_>,
    key: &[u8],
) -> Result<Vec<T>, CoreError> {
    match p.get(Table::Account, key)? {
        Some(v) => serde_json::from_slice(&v).map_err(|_| err("damaged state")),
        None => Ok(Vec::new()),
    }
}

pub(crate) fn save_list<T: Serialize>(
    p: &Provider<'_>,
    key: &[u8],
    list: &[T],
) -> Result<(), CoreError> {
    let value = if list.is_empty() {
        None
    } else {
        Some(serde_json::to_vec(list).map_err(|_| err("state"))?)
    };
    p.set(Table::Account, key, value)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantedIdentity {
    pub persona: Uuid,
    pub key_version: u32,
    /// The Ed25519 private key's seed, base64url.
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantedPin {
    pub actor_uri: String,
    pub key: String,
    pub key_version: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAnnouncement {
    pub device: Uuid,
    pub personas: Vec<PersonaDevice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaDevice {
    pub persona: Uuid,
    pub device: Uuid,
}

// ---- This device's state ------------------------------------------------------------

struct AccountDevice {
    id: Uuid,
    key: SigningKey,
}

fn account_device(store: &Store) -> Result<AccountDevice, CoreError> {
    if let Some(v) = store.get(Table::Account, K_DEVICE)? {
        if v.len() != 48 {
            return Err(err("the stored account device key is damaged"));
        }
        let seed: Zeroizing<[u8; 32]> =
            Zeroizing::new(v[16..].try_into().map_err(|_| err("damaged key"))?);
        return Ok(AccountDevice {
            id: Uuid::from_slice(&v[..16]).map_err(|_| err("damaged device id"))?,
            key: SigningKey::from_bytes(&seed),
        });
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut()).map_err(|e| err(format!("no randomness: {e}")))?;
    let dev = AccountDevice {
        id: Uuid::new_v4(),
        key: SigningKey::from_bytes(&seed),
    };
    let mut value = Zeroizing::new(dev.id.as_bytes().to_vec());
    value.extend_from_slice(seed.as_ref());
    store.apply(&[Op::Put {
        table: Table::Account,
        key: K_DEVICE,
        value: &value,
    }])?;
    Ok(dev)
}

impl AccountDevice {
    fn credential(&self) -> CredentialWithKey {
        CredentialWithKey {
            credential: BasicCredential::new(
                format!("{CREDENTIAL_PREFIX}{}", self.id).into_bytes(),
            )
            .into(),
            signature_key: self.key.verifying_key().to_bytes().to_vec().into(),
        }
    }

    fn signer(&self) -> DeviceSigner<'_> {
        DeviceSigner(&self.key)
    }
}

/// The account device id a personal-group credential names.
fn device_of(credential: &Credential) -> Option<Uuid> {
    let basic = BasicCredential::try_from(credential.clone()).ok()?;
    let text = std::str::from_utf8(basic.identity()).ok()?;
    Uuid::parse_str(text.strip_prefix(CREDENTIAL_PREFIX)?).ok()
}

fn group_id(store: &Store) -> Result<Option<Vec<u8>>, CoreError> {
    Ok(store.get(Table::Account, K_GROUP)?.map(|v| v.to_vec()))
}

/// This store's account device id, if it has one yet.
fn this_device_id(p: &Provider<'_>) -> Result<Option<Uuid>, CoreError> {
    Ok(p.get(Table::Account, K_DEVICE)?
        .and_then(|v| v.get(..16).and_then(|b| Uuid::from_slice(b).ok())))
}

/// This device's id among the login's devices, once it's in the device
/// group.
pub(crate) fn account_device_id(store: &Store) -> Result<Option<Uuid>, CoreError> {
    if group_id(store)?.is_none()
        || store.get(Table::Account, K_REMOVED)?.is_some()
        || store.get(Table::Account, K_STALE)?.is_some()
    {
        return Ok(None);
    }
    this_device_id(&Provider::new(store))
}

fn u64_of(v: Option<Vec<u8>>) -> u64 {
    v.and_then(|b| b.try_into().ok())
        .map(u64::from_be_bytes)
        .unwrap_or(0)
}

fn own_seqs(p: &Provider<'_>) -> Result<Vec<u64>, CoreError> {
    Ok(match p.get(Table::Account, K_OWN)? {
        Some(v) => serde_json::from_slice(&v).map_err(|_| err("damaged state"))?,
        None => Vec::new(),
    })
}

/// Notes a message this device posted, so catching up skips it (MLS can't
/// decrypt one's own messages).
fn record_own(store: &Store, seq: u64) -> Result<(), CoreError> {
    let p = Provider::new(store);
    p.atomically(|p| {
        let mut own = own_seqs(p)?;
        own.push(seq);
        p.set(
            Table::Account,
            K_OWN,
            Some(serde_json::to_vec(&own).map_err(|_| err("state"))?),
        )
    })
}

/// Past epochs whose secrets a member keeps. A message for an epoch can
/// reach the log after a commit for it from a member who hadn't seen it yet
/// (both are valid for that epoch), so members read a little behind.
pub(crate) const PAST_EPOCHS: usize = 5;
/// Application messages are padded to a multiple of this many bytes, so
/// ciphertext sizes say less about what was written.
pub(crate) const PADDING: usize = 256;

pub(crate) fn create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .max_past_epochs(PAST_EPOCHS)
        .padding_size(PADDING)
        .capabilities(mls::capabilities())
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .build()
}

pub(crate) fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .max_past_epochs(PAST_EPOCHS)
        .padding_size(PADDING)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .build()
}

fn load_group(p: &Provider<'_>, id: &[u8]) -> Result<MlsGroup, CoreError> {
    MlsGroup::load(p.storage(), &openmls::prelude::GroupId::from_slice(id))
        .map_err(|e| mls_err("loading the device group", e))?
        .ok_or_else(|| err("the device group's state is missing from this store"))
}

fn encode(m: &MlsMessageOut) -> Result<Vec<u8>, CoreError> {
    m.tls_serialize_detached()
        .map_err(|e| mls_err("encoding", e))
}

/// 6 words both devices show after linking: from the epoch authenticator
/// of the epoch the new device joins.
fn confirmation(group: &MlsGroup) -> Vec<&'static str> {
    words::words(&words::labelled_hash(
        "wr link confirm v1",
        &[group.epoch_authenticator().as_slice()],
    ))
}

// ---- Catching up ----------------------------------------------------------------------

/// What [`catch_up`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CatchUp {
    pub processed: usize,
    /// Identity keys received from another device.
    pub granted: usize,
    /// This device was removed from the login's devices; its keys are gone.
    pub removed: bool,
    /// This device was away longer than the server keeps the device group's
    /// messages: it must be linked again, keeping its keys (2k).
    pub stale: bool,
}

/// Reads and applies everything new in the personal device group: other
/// devices joining or leaving, keys granted, devices announced. Each
/// message's effects land in one batch with the MLS state change, so a
/// crash never applies half of one.
pub async fn catch_up(store: &Store, client: &Client) -> Result<CatchUp, CoreError> {
    let mut report = CatchUp::default();
    let Some(gid) = group_id(store)? else {
        return Ok(report);
    };
    if store.get(Table::Account, K_REMOVED)?.is_some() {
        report.removed = true;
        return Ok(report);
    }
    if store.get(Table::Account, K_STALE)?.is_some() {
        report.stale = true;
        return Ok(report);
    }
    if store.get(Table::Account, K_PENDING)?.is_some() {
        // A link waiting for the user to compare words: take nothing yet.
        return Ok(report);
    }
    let gid_text = b64(&gid);
    loop {
        let after = u64_of(store.get(Table::Account, K_SEQ)?.map(|v| v.to_vec()));
        let page = match client
            .group_messages(
                &gid_text,
                &MessagesQuery {
                    after: Some(after),
                    limit: Some(100),
                },
            )
            .await
        {
            Ok(page) => page,
            Err(e) if e.status() == Some(410) => {
                store.apply(&[Op::Put {
                    table: Table::Account,
                    key: K_STALE,
                    value: &[1],
                }])?;
                report.stale = true;
                return Ok(report);
            }
            Err(e) => return Err(e),
        };
        for m in &page.messages {
            let bytes = unb64(&m.message).ok_or_else(|| err("server sent bad base64"))?;
            let provider = Provider::new(store);
            let outcome = provider.atomically(|p| {
                let own = own_seqs(p)?;
                let outcome = if own.contains(&m.seq) {
                    Applied::default()
                } else {
                    apply(p, &gid, &bytes)?
                };
                let own: Vec<u64> = own.into_iter().filter(|s| *s > m.seq).collect();
                p.set(
                    Table::Account,
                    K_OWN,
                    Some(serde_json::to_vec(&own).map_err(|_| err("state"))?),
                )?;
                p.set(Table::Account, K_SEQ, Some(m.seq.to_be_bytes().to_vec()))?;
                Ok(outcome)
            });
            let outcome =
                outcome.map_err(|e| err(format!("device group message {}: {e}", m.seq)))?;
            report.processed += 1;
            report.granted += outcome.granted;
            if outcome.removed {
                report.removed = true;
                return Ok(report);
            }
        }
        if !page.more {
            return Ok(report);
        }
    }
}

#[derive(Default)]
struct Applied {
    granted: usize,
    removed: bool,
}

/// Applies one message from the group's log, within the caller's batch.
fn apply(p: &Provider<'_>, gid: &[u8], bytes: &[u8]) -> Result<Applied, CoreError> {
    let mut group = load_group(p, gid)?;
    let message = MlsMessageIn::tls_deserialize_exact(bytes)
        .map_err(|e| mls_err("decoding", e))?
        .try_into_protocol_message()
        .map_err(|e| mls_err("decoding", e))?;
    let processed = group
        .process_message(p, message)
        .map_err(|e| mls_err("processing", e))?;
    let sender = device_of(processed.credential());
    let mut out = Applied::default();
    match processed.into_content() {
        ProcessedMessageContent::StagedCommitMessage(staged) => {
            let removed = staged.self_removed();
            group
                .merge_staged_commit(p, *staged)
                .map_err(|e| mls_err("merging", e))?;
            if removed {
                forget_keys(p)?;
                out.removed = true;
            }
        }
        ProcessedMessageContent::ApplicationMessage(app) => {
            let plain = Zeroizing::new(app.into_bytes());
            let payload: Payload =
                serde_json::from_slice(&plain).map_err(|_| err("unreadable device message"))?;
            match payload {
                Payload::DeviceGrant {
                    identities,
                    pins,
                    devices,
                } => {
                    out.granted = take_grant(p, &identities, &pins)?;
                    let me = this_device_id(p)?;
                    for d in devices {
                        // Never let another device describe this one; and
                        // what a device announced itself wins.
                        if Some(d.device) == me
                            || p.get(Table::Siblings, d.device.as_bytes())?.is_some()
                        {
                            continue;
                        }
                        p.set(
                            Table::Siblings,
                            d.device.as_bytes(),
                            Some(serde_json::to_vec(&d.personas).map_err(|_| err("state"))?),
                        )?;
                    }
                }
                Payload::DeviceKeys { device, personas } => {
                    // A device speaks only for itself.
                    if sender != Some(device) {
                        return Err(err("a device announced keys for another device"));
                    }
                    p.set(
                        Table::Siblings,
                        device.as_bytes(),
                        Some(serde_json::to_vec(&personas).map_err(|_| err("state"))?),
                    )?;
                }
                Payload::HistoryRequest { session, from, to } => {
                    let Some(device) = sender else {
                        return Ok(out);
                    };
                    if Some(device) == this_device_id(p)? || from > to {
                        return Ok(out);
                    }
                    let mut asks: Vec<Ask> = load_list(p, K_ASKS)?;
                    asks.retain(|a| !(a.device == device && a.session == session));
                    asks.push(Ask {
                        device,
                        session,
                        from,
                        to,
                        at_ms: keys::now_ms(),
                    });
                    save_list(p, K_ASKS, &asks)?;
                }
                Payload::HistoryBundle(bundle) => {
                    if Some(bundle.for_device) == this_device_id(p)? {
                        let mut inbox: Vec<HistoryBundle> = load_list(p, K_INBOX)?;
                        inbox.push(bundle);
                        save_list(p, K_INBOX, &inbox)?;
                    } else {
                        // Another device answered: this one needn't.
                        let mut asks: Vec<Ask> = load_list(p, K_ASKS)?;
                        asks.retain(|a| {
                            !(a.device == bundle.for_device && a.session == bundle.session)
                        });
                        save_list(p, K_ASKS, &asks)?;
                    }
                }
            }
        }
        // Quill sends no standalone proposals.
        _ => {}
    }
    Ok(out)
}

/// Saves granted identity keys this device lacks or holds older versions
/// of, and pins it hasn't seen. Returns how many keys it took.
fn take_grant(
    p: &Provider<'_>,
    identities: &[GrantedIdentity],
    pins: &[GrantedPin],
) -> Result<usize, CoreError> {
    let mut taken = 0;
    for g in identities {
        let have = p
            .get(Table::Identities, g.persona.as_bytes())?
            .and_then(|v| keys::identity_version_of(&v));
        if have.is_some_and(|v| v >= g.key_version) {
            continue;
        }
        let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
            unb64(&g.key)
                .and_then(|k| k.try_into().ok())
                .ok_or_else(|| err("a granted key is malformed"))?,
        );
        let value = keys::identity_value(g.key_version, &seed);
        p.set(
            Table::Identities,
            g.persona.as_bytes(),
            Some(value.to_vec()),
        )?;
        taken += 1;
    }
    for pin in pins {
        if p.get(Table::Pins, pin.actor_uri.as_bytes())?.is_none() {
            let key = unb64(&pin.key).ok_or_else(|| err("a granted pin is malformed"))?;
            let mut value = pin.key_version.to_be_bytes().to_vec();
            value.extend_from_slice(&key);
            p.set(Table::Pins, pin.actor_uri.as_bytes(), Some(value))?;
        }
    }
    Ok(taken)
}

/// After removal: the personas' keys go, so this device can't act for them.
fn forget_keys(p: &Provider<'_>) -> Result<(), CoreError> {
    for table in [Table::Identities, Table::Devices] {
        for (k, _) in p_scan(p, table)? {
            p.set(table, &k, None)?;
        }
    }
    p.set(Table::Account, K_REMOVED, Some(vec![1]))
}

fn p_scan(p: &Provider<'_>, table: Table) -> Result<mls::Entries, CoreError> {
    p.scan(table)
}

// ---- Sending -------------------------------------------------------------------------------

/// Posts a commit made by `make` (which leaves it pending), merging it once
/// the server accepts it. Losing an epoch race clears it, catches up, and
/// tries again.
async fn commit<T>(
    store: &Store,
    client: &Client,
    gid: &[u8],
    make: impl Fn(&Provider<'_>, &mut MlsGroup, &AccountDevice) -> Result<(MlsMessageOut, T), CoreError>,
) -> Result<(Posted, T, MlsGroup), CoreError> {
    let me = account_device(store)?;
    for _ in 0..RETRIES {
        let provider = Provider::new(store);
        let mut group = load_group(&provider, gid)?;
        let (message, extra) = provider.atomically(|p| make(p, &mut group, &me))?;
        let body = PostMessage {
            message: b64(&encode(&message)?),
            blobs: Vec::new(),
        };
        match client.post_group_message(&b64(gid), &body).await {
            Ok(posted) => {
                provider.atomically(|p| {
                    group
                        .merge_pending_commit(p)
                        .map_err(|e| mls_err("merging", e))
                })?;
                record_own(store, posted.seq)?;
                return Ok((posted, extra, group));
            }
            Err(e) if conflict(&e) => {
                provider.atomically(|p| {
                    group
                        .clear_pending_commit(p.storage())
                        .map_err(|e| mls_err("clearing", e))
                })?;
                catch_up(store, client).await?;
            }
            Err(e) => {
                let _ = provider.atomically(|p| {
                    group
                        .clear_pending_commit(p.storage())
                        .map_err(|e| mls_err("clearing", e))
                });
                return Err(e);
            }
        }
    }
    Err(err("the device group kept changing; try again"))
}

/// Sends an application message to the personal group.
async fn send(
    store: &Store,
    client: &Client,
    gid: &[u8],
    payload: &Payload,
) -> Result<(), CoreError> {
    send_with(store, client, gid, payload, &[]).await
}

/// Sends a payload to the device group, if this device is in one, with
/// the blobs it carries.
pub(crate) async fn send_payload(
    store: &Store,
    client: &Client,
    payload: &Payload,
    blobs: &[Uuid],
) -> Result<(), CoreError> {
    if store.get(Table::Account, K_REMOVED)?.is_some() {
        return Err(err("this device was removed from this login's devices"));
    }
    let gid = group_id(store)?.ok_or_else(|| err("this device has no device group"))?;
    send_with(store, client, &gid, payload, blobs).await
}

/// The device group's id, base64url, if this device is in one.
pub(crate) fn device_group(store: &Store) -> Result<Option<String>, CoreError> {
    Ok(group_id(store)?.map(|g| b64(&g)))
}

async fn send_with(
    store: &Store,
    client: &Client,
    gid: &[u8],
    payload: &Payload,
    blobs: &[Uuid],
) -> Result<(), CoreError> {
    let me = account_device(store)?;
    let plain = Zeroizing::new(serde_json::to_vec(payload).map_err(|_| err("encoding"))?);
    for _ in 0..RETRIES {
        let provider = Provider::new(store);
        let mut group = load_group(&provider, gid)?;
        let message = provider.atomically(|p| {
            group
                .create_message(p, &me.signer(), &plain)
                .map_err(|e| mls_err("encrypting", e))
        })?;
        let body = PostMessage {
            message: b64(&encode(&message)?),
            blobs: blobs.to_vec(),
        };
        match client.post_group_message(&b64(gid), &body).await {
            Ok(posted) => return record_own(store, posted.seq),
            Err(e) if conflict(&e) => {
                catch_up(store, client).await?;
            }
            Err(e) => return Err(e),
        }
    }
    Err(err("the device group kept changing; try again"))
}

/// The personal group: this store's, or a new one if the login has none.
async fn ensure_group(store: &Store, client: &Client) -> Result<Vec<u8>, CoreError> {
    if let Some(id) = group_id(store)? {
        return Ok(id);
    }
    if client.device_group().await?.is_some() {
        return Err(err(
            "this login's devices already have a group this device isn't in; link this device with `quill device link`",
        ));
    }
    let me = account_device(store)?;
    let provider = Provider::new(store);
    let group = provider.atomically(|p| {
        MlsGroup::new(p, &me.signer(), &create_config(), me.credential())
            .map_err(|e| mls_err("creating the device group", e))
    })?;
    let id = group.group_id().as_slice().to_vec();
    client
        .create_group(&CreateGroup {
            group_id: b64(&id),
            kind: "personal".into(),
        })
        .await?;
    store.apply(&[
        Op::Put {
            table: Table::Account,
            key: K_GROUP,
            value: &id,
        },
        Op::Put {
            table: Table::Account,
            key: K_SEQ,
            value: &0u64.to_be_bytes(),
        },
    ])?;
    Ok(id)
}

/// Tells the login's other devices which persona devices are this one's,
/// if that changed since last time.
pub async fn announce(store: &Store, client: &Client) -> Result<(), CoreError> {
    let Some(gid) = group_id(store)? else {
        return Ok(());
    };
    if store.get(Table::Account, K_REMOVED)?.is_some()
        || store.get(Table::Account, K_STALE)?.is_some()
    {
        return Ok(());
    }
    let personas: Vec<PersonaDevice> = keys::own_persona_devices(store)?
        .into_iter()
        .map(|(persona, device)| PersonaDevice { persona, device })
        .collect();
    let digest = Sha256::digest(serde_json::to_vec(&personas).map_err(|_| err("encoding"))?);
    if store
        .get(Table::Account, K_ANNOUNCED)?
        .is_some_and(|v| v.as_slice() == digest.as_slice())
    {
        return Ok(());
    }
    let me = account_device(store)?;
    send(
        store,
        client,
        &gid,
        &Payload::DeviceKeys {
            device: me.id,
            personas,
        },
    )
    .await?;
    store.apply(&[Op::Put {
        table: Table::Account,
        key: K_ANNOUNCED,
        value: digest.as_slice(),
    }])?;
    Ok(())
}

// ---- Linking: the new device ---------------------------------------------------------------

/// A link waiting for a device already in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingLink {
    pub link_id: String,
    /// `quill-link:1:HOST:LINK_ID:FINGERPRINT`
    pub code: String,
    /// The fingerprint's 6 words, for typing the code instead.
    pub words: Vec<&'static str>,
}

/// On the new device: posts a KeyPackage for the personal group and
/// returns the code to show.
pub async fn start_link(store: &Store, client: &Client) -> Result<PendingLink, CoreError> {
    if store.get(Table::Account, K_REMOVED)?.is_some()
        || store.get(Table::Account, K_STALE)?.is_some()
    {
        // Removed earlier, or fell behind: start over as a new device,
        // dropping the old group's MLS state too. A device that fell behind
        // keeps its persona keys and sessions.
        if let Some(old) = group_id(store)? {
            let provider = Provider::new(store);
            provider.atomically(|p| {
                if let Ok(mut g) = load_group(p, &old) {
                    g.delete(p.storage())
                        .map_err(|e| mls_err("forgetting the old group", e))?;
                }
                Ok(())
            })?;
        }
        let mut ops = Vec::new();
        let keys: Vec<Vec<u8>> = [
            K_DEVICE,
            K_GROUP,
            K_SEQ,
            K_OWN,
            K_PENDING,
            K_REMOVED,
            K_ANNOUNCED,
            K_STALE,
            K_ASKS,
            K_INBOX,
        ]
        .iter()
        .map(|k| k.to_vec())
        .collect();
        for k in &keys {
            ops.push(Op::Delete {
                table: Table::Account,
                key: k,
            });
        }
        let siblings: Vec<Vec<u8>> = store
            .scan(Table::Siblings, b"")?
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        for k in &siblings {
            ops.push(Op::Delete {
                table: Table::Siblings,
                key: k,
            });
        }
        store.apply(&ops)?;
    } else if group_id(store)?.is_some() {
        return Err(err("this device is already one of this login's devices"));
    }
    let me = account_device(store)?;
    let provider = Provider::new(store);
    let (not_before, not_after) = mls::lifetime(keys::now_ms() / 1000);
    let package = provider.atomically(|p| {
        let bundle = KeyPackage::builder()
            .key_package_lifetime(Lifetime::init(not_before, not_after))
            .leaf_node_capabilities(mls::capabilities())
            .build(CIPHERSUITE, p, &me.signer(), me.credential())
            .map_err(|e| mls_err("making a KeyPackage", e))?;
        encode(&MlsMessageOut::from(bundle.key_package().clone()))
    })?;
    let started = client
        .start_link(&StartLink {
            key_package: b64(&package),
        })
        .await?;
    let fingerprint = Sha256::digest(&package);
    Ok(PendingLink {
        code: format!(
            "{LINK_PREFIX}{}:{}:{}",
            client.host(),
            started.link_id,
            b64(&fingerprint)
        ),
        words: words::words(&fingerprint),
        link_id: started.link_id,
    })
}

/// On the new device: waits for the answer and joins the group. Returns
/// the confirmation words; the keys sent along aren't taken until
/// [`finish_link`].
pub async fn wait_for_link(
    store: &Store,
    client: &Client,
    link_id: &str,
    poll: std::time::Duration,
    timeout: std::time::Duration,
) -> Result<Vec<&'static str>, CoreError> {
    let deadline = std::time::Instant::now() + timeout;
    let (welcome, from_seq) = loop {
        let mailbox = client.read_link(link_id).await.map_err(|e| {
            if e.status() == Some(404) {
                err("the link expired before it was approved; start again")
            } else {
                e
            }
        })?;
        if let (Some(w), Some(s)) = (mailbox.welcome, mailbox.from_seq) {
            break (w, s);
        }
        if std::time::Instant::now() >= deadline {
            return Err(err("no device approved the link in time; start again"));
        }
        tokio::time::sleep(poll).await;
    };
    let bytes = unb64(&welcome).ok_or_else(|| err("server sent bad base64"))?;
    let welcome = match MlsMessageIn::tls_deserialize_exact(bytes)
        .map_err(|e| mls_err("decoding the Welcome", e))?
        .extract()
    {
        MlsMessageBodyIn::Welcome(w) => w,
        _ => return Err(err("the answer isn't a Welcome")),
    };
    let provider = Provider::new(store);
    let group = provider.atomically(|p| {
        let group = StagedWelcome::new_from_welcome(p, &join_config(), welcome, None)
            .and_then(|s| s.into_group(p))
            .map_err(|e| mls_err("joining the device group", e))?;
        p.set(
            Table::Account,
            K_GROUP,
            Some(group.group_id().as_slice().to_vec()),
        )?;
        p.set(Table::Account, K_SEQ, Some(from_seq.to_be_bytes().to_vec()))?;
        p.set(Table::Account, K_PENDING, Some(vec![1]))?;
        Ok(group)
    })?;
    Ok(confirmation(&group))
}

/// On the new device, once the user confirms the words: takes the keys,
/// then sets up each persona as `keys init` and `keys publish` would.
/// Returns the personas set up.
pub async fn finish_link(store: &Store, client: &Client) -> Result<Vec<String>, CoreError> {
    store.apply(&[Op::Delete {
        table: Table::Account,
        key: K_PENDING,
    }])?;
    catch_up(store, client).await?;
    let mut done = Vec::new();
    for persona in client.personas().await? {
        if keys::load_identity(store, persona.id)?.is_none() {
            continue;
        }
        keys::init(store, client, &persona, false).await?;
        keys::publish(store, client, &persona, keys::POOL_TARGET).await?;
        done.push(persona.handle);
    }
    announce(store, client).await?;
    Ok(done)
}

/// On the new device, when the words didn't match: forgets the group it
/// was given without taking anything from it.
pub fn abandon_link(store: &Store) -> Result<(), CoreError> {
    Ok(store.apply(&[
        Op::Delete {
            table: Table::Account,
            key: K_GROUP,
        },
        Op::Delete {
            table: Table::Account,
            key: K_SEQ,
        },
        Op::Delete {
            table: Table::Account,
            key: K_PENDING,
        },
    ])?)
}

// ---- Linking: the device already in use ----------------------------------------------------

/// A link code as typed: the full code, or the link id and 6 words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkCode {
    pub link_id: String,
    check: Check,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Check {
    Fingerprint(Vec<u8>),
    Words(String),
}

impl std::str::FromStr for LinkCode {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, CoreError> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix(LINK_PREFIX) {
            // HOST may contain a colon (a port), so split from the right.
            let mut parts = rest.rsplitn(3, ':');
            let fp = parts.next().and_then(unb64);
            let link = parts.next();
            return match (link, fp) {
                (Some(link), Some(fp)) if fp.len() == 32 && !link.is_empty() => Ok(Self {
                    link_id: link.to_owned(),
                    check: Check::Fingerprint(fp),
                }),
                _ => Err(err("that isn't a complete link code")),
            };
        }
        let mut parts = s.split_whitespace();
        let link = parts.next().unwrap_or_default();
        let typed: Vec<&str> = parts.collect();
        if link.is_empty() || typed.len() != 6 {
            return Err(err(
                "give the whole quill-link: code, or the link id followed by its 6 words",
            ));
        }
        Ok(Self {
            link_id: link.to_owned(),
            check: Check::Words(typed.join(" ")),
        })
    }
}

/// What [`approve`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approved {
    /// The new device's id among the login's devices.
    pub device: Uuid,
    /// The words the new device should show too.
    pub confirm: Vec<&'static str>,
    /// Identity keys sent to it.
    pub granted: usize,
}

/// On a device already in use: checks the new device's KeyPackage against
/// the code, adds it to the personal group, sends it the identity keys and
/// pins, and answers the link.
pub async fn approve(
    store: &Store,
    client: &Client,
    code: &LinkCode,
) -> Result<Approved, CoreError> {
    let gid = ensure_group(store, client).await?;
    let caught = catch_up(store, client).await?;
    if caught.removed {
        return Err(err("this device was removed from this login's devices"));
    }
    if caught.stale {
        return Err(fell_behind());
    }
    let mailbox = client.read_link(&code.link_id).await.map_err(|e| {
        if e.status() == Some(404) {
            err("no such link: it may have expired")
        } else {
            e
        }
    })?;
    let package = unb64(&mailbox.key_package).ok_or_else(|| err("server sent bad base64"))?;
    let fingerprint = Sha256::digest(&package);
    let ok = match &code.check {
        Check::Fingerprint(fp) => fp.as_slice() == fingerprint.as_slice(),
        Check::Words(w) => words::matches(&fingerprint, w),
    };
    if !ok {
        return Err(err(
            "the new device's key doesn't match the code: don't link it. The server may have swapped it.",
        ));
    }
    let package = match MlsMessageIn::tls_deserialize_exact(&package)
        .map_err(|e| mls_err("decoding", e))?
        .extract()
    {
        MlsMessageBodyIn::KeyPackage(k) => k
            .validate(Provider::new(store).crypto(), ProtocolVersion::Mls10)
            .map_err(|e| mls_err("the new device's KeyPackage", e))?,
        _ => return Err(err("the link holds no KeyPackage")),
    };
    let device = device_of(package.leaf_node().credential())
        .ok_or_else(|| err("the new device's credential isn't a Quill device"))?;

    let (posted, welcome, group) = commit(store, client, &gid, |p, group, me| {
        let (commit, welcome, _) = group
            .add_members(p, &me.signer(), std::slice::from_ref(&package))
            .map_err(|e| mls_err("adding the device", e))?;
        Ok((commit, welcome))
    })
    .await?;
    let confirm = confirmation(&group);

    // The keys, for the new device only to read (MLS encrypts them to the
    // group, which now holds it and this login's other devices).
    let identities: Vec<GrantedIdentity> = keys::export_identities(store)?
        .into_iter()
        .map(|(persona, key_version, seed)| GrantedIdentity {
            persona,
            key_version,
            key: b64(seed.as_ref()),
        })
        .collect();
    let granted = identities.len();
    let pins = store
        .scan(Table::Pins, b"")?
        .into_iter()
        .filter(|(_, v)| v.len() == 36)
        .map(|(k, v)| GrantedPin {
            actor_uri: String::from_utf8_lossy(&k).into_owned(),
            key: b64(&v[4..]),
            key_version: u32::from_be_bytes([v[0], v[1], v[2], v[3]]),
        })
        .collect();
    let me = account_device(store)?;
    let mut known = vec![DeviceAnnouncement {
        device: me.id,
        personas: keys::own_persona_devices(store)?
            .into_iter()
            .map(|(persona, device)| PersonaDevice { persona, device })
            .collect(),
    }];
    for (k, v) in store.scan(Table::Siblings, b"")? {
        if let (Ok(device), Ok(personas)) = (Uuid::from_slice(&k), serde_json::from_slice(&v)) {
            known.push(DeviceAnnouncement { device, personas });
        }
    }
    send(
        store,
        client,
        &gid,
        &Payload::DeviceGrant {
            identities,
            pins,
            devices: known,
        },
    )
    .await?;
    client
        .reply_link(
            &code.link_id,
            &LinkReply {
                welcome: b64(&encode(&welcome)?),
                from_seq: posted.seq,
            },
        )
        .await?;
    announce(store, client).await?;
    Ok(Approved {
        device,
        confirm,
        granted,
    })
}

// ---- Listing and removing ------------------------------------------------------------------

/// One of the login's devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedDevice {
    pub id: Uuid,
    pub this_device: bool,
    /// Its persona devices, as it announced them (or, for this device, as
    /// it holds them).
    pub personas: Vec<PersonaDevice>,
}

/// The login's devices, after catching up.
pub async fn list(store: &Store, client: &Client) -> Result<Vec<ListedDevice>, CoreError> {
    let caught = catch_up(store, client).await?;
    if caught.removed {
        return Err(err("this device was removed from this login's devices"));
    }
    if caught.stale {
        return Err(fell_behind());
    }
    let Some(gid) = group_id(store)? else {
        return Ok(Vec::new());
    };
    let me = account_device(store)?;
    let provider = Provider::new(store);
    let group = load_group(&provider, &gid)?;
    group
        .members()
        .filter_map(|m| device_of(&m.credential))
        .map(|id| {
            let personas = if id == me.id {
                keys::own_persona_devices(store)?
                    .into_iter()
                    .map(|(persona, device)| PersonaDevice { persona, device })
                    .collect()
            } else {
                store
                    .get(Table::Siblings, id.as_bytes())?
                    .map(|v| serde_json::from_slice(&v).map_err(|_| err("damaged state")))
                    .transpose()?
                    .unwrap_or_default()
            };
            Ok(ListedDevice {
                id,
                this_device: id == me.id,
                personas,
            })
        })
        .collect()
}

/// What [`remove`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    /// Persona devices revoked in the directory.
    pub revoked: Vec<Uuid>,
}

/// Takes another of the login's devices out of the personal group (which
/// rotates the group's keys) and revokes its persona devices.
pub async fn remove(
    store: &Store,
    client: &Client,
    device: Uuid,
    reason: RevocationReason,
) -> Result<Removed, CoreError> {
    let devices = list(store, client).await?;
    let target = devices
        .iter()
        .find(|d| d.id == device)
        .ok_or_else(|| err(format!("{device} isn't one of this login's devices")))?;
    if target.this_device {
        return Err(err("remove this device from another one"));
    }
    let personas = client.personas().await?;
    let mut revoked = Vec::new();
    // A device linked again after falling behind (2k) announces the persona
    // devices its old entry did: those stay.
    let in_use: Vec<Uuid> = devices
        .iter()
        .filter(|d| d.id != device)
        .flat_map(|d| d.personas.iter().map(|p| p.device))
        .collect();
    for pd in target
        .personas
        .iter()
        .filter(|p| !in_use.contains(&p.device))
    {
        let Some(persona) = personas.iter().find(|p| p.id == pd.persona) else {
            continue;
        };
        match keys::revoke(store, client, persona, pd.device, reason).await {
            Ok(_) => revoked.push(pd.device),
            // Already revoked, or gone: nothing to do.
            Err(e) if matches!(e.status(), Some(404 | 409)) => {}
            Err(e) => return Err(e),
        }
    }
    let gid = group_id(store)?.ok_or_else(|| err("no device group"))?;
    commit(store, client, &gid, |p, group, me| {
        let leaf: LeafNodeIndex = group
            .members()
            .find(|m| device_of(&m.credential) == Some(device))
            .map(|m| m.index)
            .ok_or_else(|| err("that device already left"))?;
        let (commit, _, _) = group
            .remove_members(p, &me.signer(), &[leaf])
            .map_err(|e| mls_err("removing the device", e))?;
        p.set(Table::Siblings, device.as_bytes(), None)?;
        Ok((commit, ()))
    })
    .await?;
    Ok(Removed { revoked })
}

// ---- Upkeep ---------------------------------------------------------------------------------

/// Refill below this many one-time KeyPackages.
pub const POOL_LOW: u32 = 10;

/// What [`maintain`] did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Upkeep {
    pub caught_up: CatchUp,
    /// Personas whose pools were refilled, with how many were added.
    pub refilled: Vec<(String, u32)>,
    /// Groups dropped for requests that were declined, withdrawn or expired.
    pub requests_dropped: usize,
    /// What syncing the login's sessions did.
    pub sessions: crate::sessions::SyncReport,
    /// History sent to and received from the login's other devices (2k).
    pub history: crate::history_sync::HistoryReport,
    /// Why history couldn't be synced this time, if it couldn't.
    pub history_problem: Option<String>,
}

/// Runs before a command: catches up on the device group, refills low
/// KeyPackage pools (and renews last-resort packages near expiry), and
/// announces this device's persona devices if they changed.
pub async fn maintain(store: &Store, client: &Client) -> Result<Upkeep, CoreError> {
    let mut report = Upkeep {
        caught_up: catch_up(store, client).await?,
        ..Upkeep::default()
    };
    if report.caught_up.removed {
        return Ok(report);
    }
    let personas: Vec<Persona> = client.personas().await?;
    for persona in &personas {
        let Some(device) = keys::load_device(store, persona.id)?.and_then(|d| d.id) else {
            continue;
        };
        let status = match client.key_package_status(persona.id, device).await {
            Ok(s) => s,
            // Revoked or superseded: `keys init` sorts that out.
            Err(e) if matches!(e.status(), Some(404 | 409)) => continue,
            Err(e) => return Err(e),
        };
        let now = keys::now_ms() / 1000;
        let renew = status
            .last_resort_until
            .is_none_or(|t| t.saturating_sub(now) < 7 * 86_400);
        if status.available < POOL_LOW || renew {
            let out = keys::publish(store, client, persona, keys::POOL_TARGET).await?;
            if out.uploaded > 0 || out.last_resort_renewed {
                report.refilled.push((persona.handle.clone(), out.uploaded));
            }
        }
    }
    announce(store, client).await?;
    report.requests_dropped = crate::requests::tidy(store, client).await?;
    report.sessions = crate::sessions::sync(store, client).await?;
    // History waits for the next run if it can't go now (a full quota, an
    // unreachable blob): it never stops the rest.
    match crate::history_sync::run(store, client).await {
        Ok(h) => report.history = h,
        Err(e) => report.history_problem = Some(e.to_string()),
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_codes_parse_both_ways() {
        let fp = b64(&[7u8; 32]);
        let code: LinkCode = format!("quill-link:1:writers.example:8090:abc123:{fp}")
            .parse()
            .unwrap();
        assert_eq!(code.link_id, "abc123");
        assert_eq!(code.check, Check::Fingerprint(vec![7; 32]));
        let typed: LinkCode = "abc123 one two three four five six".parse().unwrap();
        assert_eq!(typed.link_id, "abc123");
        for bad in [
            "quill-link:1:host:abc",
            "abc123 one two",
            "",
            "quill-link:1:h:id:short",
        ] {
            assert!(bad.parse::<LinkCode>().is_err(), "{bad}");
        }
    }
}
