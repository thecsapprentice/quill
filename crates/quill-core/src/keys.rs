//! Persona identity keys and this device's keys (2b).
//!
//! Every persona that takes part in sessions has its own Ed25519 identity
//! key, and each device has its own device key per persona, certified by
//! that identity key. Nothing is shared between personas, so keys can't link
//! them. Private keys live only in the encrypted store; the server publishes
//! the public halves, which this module also checks for other personas.

use ed25519_dalek::SigningKey;
use quill_certs::{DeviceCertificate, DeviceRevocation, Expected, RevocationReason, fingerprint};
use quill_proto::{
    api::Persona,
    keys::{DeviceInfo, PersonaKeys, RegisterDevice, RevokeDevice, SetIdentityKey, b64, unb64},
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    Client, CoreError, Store,
    store::kv::{Op, Table},
};

const DAY_MS: u64 = 86_400_000;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// The start of `ms`'s UTC day. Certificates start there, so certificates
/// made together for different personas can't be matched by time.
pub fn day_start(ms: u64) -> u64 {
    ms - ms % DAY_MS
}

fn keys_err(msg: impl Into<String>) -> CoreError {
    CoreError::Keys(msg.into())
}

fn generate() -> Result<SigningKey, CoreError> {
    let mut seed = Zeroizing::new([0u8; 32]);
    getrandom::fill(seed.as_mut()).map_err(|e| keys_err(format!("no randomness: {e}")))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn seed_of(bytes: &[u8]) -> Result<SigningKey, CoreError> {
    let seed: Zeroizing<[u8; 32]> = Zeroizing::new(
        bytes
            .try_into()
            .map_err(|_| keys_err("a stored key is damaged"))?,
    );
    Ok(SigningKey::from_bytes(&seed))
}

/// A persona's identity key, held only on this device.
pub struct Identity {
    pub version: u32,
    key: SigningKey,
}

impl Identity {
    pub fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public())
    }

    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(self.version.to_be_bytes().to_vec());
        out.extend_from_slice(self.key.as_bytes());
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() != 36 {
            return Err(keys_err("a stored identity key is damaged"));
        }
        let mut v = [0u8; 4];
        v.copy_from_slice(&bytes[..4]);
        Ok(Self {
            version: u32::from_be_bytes(v),
            key: seed_of(&bytes[4..])?,
        })
    }
}

/// This device's key and certificate for one persona.
pub struct DeviceKey {
    /// The server's id for it, once registered.
    pub id: Option<Uuid>,
    key: SigningKey,
    pub certificate: Vec<u8>,
}

impl DeviceKey {
    pub fn public(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    /// Its MLS signature key.
    pub(crate) fn signing_key(&self) -> &SigningKey {
        &self.key
    }

    fn encode(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(self.id.unwrap_or(Uuid::nil()).as_bytes().to_vec());
        out.extend_from_slice(self.key.as_bytes());
        out.extend_from_slice(&self.certificate);
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() < 48 {
            return Err(keys_err("a stored device key is damaged"));
        }
        let id = Uuid::from_slice(&bytes[..16]).map_err(|_| keys_err("damaged device id"))?;
        Ok(Self {
            id: (!id.is_nil()).then_some(id),
            key: seed_of(&bytes[16..48])?,
            certificate: bytes[48..].to_vec(),
        })
    }
}

/// A certified device key made directly from an identity key, for tests.
#[cfg(test)]
pub(crate) fn test_device(identity: &SigningKey, version: u32, actor_uri: &str) -> DeviceKey {
    let key = generate().unwrap_or_else(|_| SigningKey::from_bytes(&[7; 32]));
    let certificate = DeviceCertificate {
        actor_uri: actor_uri.to_owned(),
        device_key: key.verifying_key().to_bytes(),
        identity_key_version: version,
        limited: false,
        not_before: day_start(now_ms()),
        not_after: None,
    }
    .issue(identity)
    .unwrap_or_default();
    DeviceKey {
        id: None,
        key,
        certificate,
    }
}

/// A persona's identity key: `(persona, version, seed)`.
pub(crate) type ExportedIdentity = (Uuid, u32, Zeroizing<[u8; 32]>);

/// Every persona identity key this device holds.
pub(crate) fn export_identities(store: &Store) -> Result<Vec<ExportedIdentity>, CoreError> {
    store
        .scan(Table::Identities, b"")?
        .into_iter()
        .map(|(k, v)| {
            let persona = Uuid::from_slice(&k).map_err(|_| keys_err("damaged identity entry"))?;
            let id = Identity::decode(&v)?;
            Ok((persona, id.version, Zeroizing::new(id.key.to_bytes())))
        })
        .collect()
}

/// An identity key's stored form, from its version and seed.
pub(crate) fn identity_value(version: u32, seed: &[u8; 32]) -> Zeroizing<Vec<u8>> {
    Identity {
        version,
        key: SigningKey::from_bytes(seed),
    }
    .encode()
}

/// The version of the identity key stored in `value`.
pub(crate) fn identity_version_of(value: &[u8]) -> Option<u32> {
    Identity::decode(value).ok().map(|i| i.version)
}

/// This device's directory device id for each persona it has keys for.
pub(crate) fn own_persona_devices(store: &Store) -> Result<Vec<(Uuid, Uuid)>, CoreError> {
    let mut out = Vec::new();
    for (k, v) in store.scan(Table::Devices, b"")? {
        let persona = Uuid::from_slice(&k).map_err(|_| keys_err("damaged device entry"))?;
        if let Some(id) = DeviceKey::decode(&v)?.id {
            out.push((persona, id));
        }
    }
    Ok(out)
}

pub fn load_identity(store: &Store, persona: Uuid) -> Result<Option<Identity>, CoreError> {
    store
        .get(Table::Identities, persona.as_bytes())?
        .map(|v| Identity::decode(&v))
        .transpose()
}

fn save_identity(store: &Store, persona: Uuid, identity: &Identity) -> Result<(), CoreError> {
    let value = identity.encode();
    Ok(store.apply(&[Op::Put {
        table: Table::Identities,
        key: persona.as_bytes(),
        value: &value,
    }])?)
}

pub fn load_device(store: &Store, persona: Uuid) -> Result<Option<DeviceKey>, CoreError> {
    store
        .get(Table::Devices, persona.as_bytes())?
        .map(|v| DeviceKey::decode(&v))
        .transpose()
}

fn save_device(store: &Store, persona: Uuid, device: &DeviceKey) -> Result<(), CoreError> {
    let value = device.encode();
    Ok(store.apply(&[Op::Put {
        table: Table::Devices,
        key: persona.as_bytes(),
        value: &value,
    }])?)
}

fn forget_device(store: &Store, persona: Uuid) -> Result<(), CoreError> {
    Ok(store.apply(&[Op::Delete {
        table: Table::Devices,
        key: persona.as_bytes(),
    }])?)
}

fn published_identity(keys: &PersonaKeys) -> Result<Option<(Vec<u8>, u32)>, CoreError> {
    match (&keys.identity_key, keys.key_version) {
        (Some(k), Some(v)) => Ok(Some((
            unb64(k).ok_or_else(|| keys_err("the server sent a malformed identity key"))?,
            v,
        ))),
        _ => Ok(None),
    }
}

/// What [`init`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitOutcome {
    pub fingerprint: String,
    pub identity_version: u32,
    /// The identity key was created (or replaced) now.
    pub new_identity: bool,
    pub device: Uuid,
    /// This device was certified now.
    pub new_device: bool,
}

/// Makes sure `persona` has an identity key and this device a certified
/// device key, creating and publishing whatever is missing. Keys are saved
/// in the store before they're sent, so a failed upload can be retried.
///
/// If the persona's identity key was made on another device, this refuses
/// unless `replace_identity`, which publishes a new key with a higher
/// version: the persona's other devices then stop working until linked
/// again, and other writers see the key change.
pub async fn init(
    store: &Store,
    client: &Client,
    persona: &Persona,
    replace_identity: bool,
) -> Result<InitOutcome, CoreError> {
    let published = client.persona_keys(persona.id).await?;
    let on_server = published_identity(&published)?;
    let local = load_identity(store, persona.id)?;

    let matches = |l: &Identity| {
        on_server
            .as_ref()
            .is_some_and(|(k, v)| *k == l.public() && *v == l.version)
    };
    let (identity, upload) = match (local, &on_server) {
        (Some(l), _) if matches(&l) => (l, false),
        // Made here earlier but never published (a failed upload).
        (Some(l), None) => (l, true),
        (_, Some((_, v))) if replace_identity => {
            let id = Identity {
                version: v + 1,
                key: generate()?,
            };
            save_identity(store, persona.id, &id)?;
            (id, true)
        }
        (_, Some(_)) => {
            return Err(keys_err(format!(
                "@{}'s identity key was made on another device. Link this device to it \
                 (coming in 2d), or replace the key with --new-identity: the persona's other \
                 devices then stop working, and other writers are told the key changed.",
                persona.handle
            )));
        }
        (None, None) => {
            let id = Identity {
                version: 1,
                key: generate()?,
            };
            save_identity(store, persona.id, &id)?;
            (id, true)
        }
    };
    if upload {
        client
            .set_identity_key(
                persona.id,
                &SetIdentityKey {
                    public_key: b64(&identity.public()),
                    key_version: identity.version,
                },
            )
            .await?;
    }
    // Our own key, pinned as other writers' clients will pin it.
    if upload {
        pin(
            store,
            &persona.actor_uri,
            &identity.public(),
            identity.version,
        )?;
    } else {
        check_pin(
            store,
            &persona.actor_uri,
            &identity.public(),
            identity.version,
        )?;
    }

    // This device: a new key if it has none or the server revoked it, and a
    // new certificate if the identity key changed since.
    let mut device = load_device(store, persona.id)?;
    if let Some(d) = &device {
        let revoked = published.devices.iter().any(|p| {
            p.revocation.is_some() && unb64(&p.device_key).as_deref() == Some(d.public().as_slice())
        });
        if revoked {
            forget_device(store, persona.id)?;
            device = None;
        }
    }
    let certified_for = device
        .as_ref()
        .and_then(|d| DeviceCertificate::decode(&d.certificate).ok())
        .map(|c| c.identity_key_version);
    let registered = device.as_ref().is_some_and(|d| {
        d.id.is_some()
            && published.devices.iter().any(|p| {
                Some(p.id) == d.id && unb64(&p.certificate).as_deref() == Some(&d.certificate[..])
            })
    });
    let new_device = !(registered && certified_for == Some(identity.version));
    let mut device = match device {
        Some(d) if certified_for == Some(identity.version) => d,
        Some(d) => DeviceKey {
            id: d.id,
            certificate: issue(&identity, &persona.actor_uri, &d.key)?,
            key: d.key,
        },
        None => {
            let key = generate()?;
            DeviceKey {
                id: None,
                certificate: issue(&identity, &persona.actor_uri, &key)?,
                key,
            }
        }
    };
    if new_device {
        save_device(store, persona.id, &device)?;
        let info = client
            .register_device(
                persona.id,
                &RegisterDevice {
                    certificate: b64(&device.certificate),
                },
            )
            .await?;
        device.id = Some(info.id);
        save_device(store, persona.id, &device)?;
    }
    Ok(InitOutcome {
        fingerprint: identity.fingerprint(),
        identity_version: identity.version,
        new_identity: upload,
        device: device.id.unwrap_or(Uuid::nil()),
        new_device,
    })
}

fn issue(identity: &Identity, actor_uri: &str, device: &SigningKey) -> Result<Vec<u8>, CoreError> {
    Ok(DeviceCertificate {
        actor_uri: actor_uri.to_owned(),
        device_key: device.verifying_key().to_bytes(),
        identity_key_version: identity.version,
        limited: false,
        not_before: day_start(now_ms()),
        not_after: None,
    }
    .issue(&identity.key)?)
}

/// Revokes one of `persona`'s devices with its identity key, which this
/// device must hold. Revoking this device also forgets its key here.
pub async fn revoke(
    store: &Store,
    client: &Client,
    persona: &Persona,
    device: Uuid,
    reason: RevocationReason,
) -> Result<DeviceInfo, CoreError> {
    let identity = load_identity(store, persona.id)?.ok_or_else(|| {
        keys_err(format!(
            "this device doesn't hold @{}'s identity key, so it can't revoke devices",
            persona.handle
        ))
    })?;
    let published = client.persona_keys(persona.id).await?;
    let target = published
        .devices
        .iter()
        .find(|d| d.id == device)
        .ok_or_else(|| keys_err(format!("@{} has no device {device}", persona.handle)))?;
    let device_key: [u8; 32] = unb64(&target.device_key)
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| keys_err("the server sent a malformed device key"))?;
    let revocation = DeviceRevocation {
        actor_uri: persona.actor_uri.clone(),
        device_key,
        revoked_at: now_ms(),
        reason,
    }
    .issue(&identity.key)?;
    let info = client
        .revoke_device(
            persona.id,
            device,
            &RevokeDevice {
                revocation: b64(&revocation),
            },
        )
        .await?;
    if load_device(store, persona.id)?.is_some_and(|d| d.public() == device_key) {
        forget_device(store, persona.id)?;
    }
    Ok(info)
}

// ---- KeyPackages (2c) ------------------------------------------------------------

/// One-time KeyPackages each device keeps on the server per persona.
pub const POOL_TARGET: u32 = 20;
/// Replace the last-resort package when it has less than this left.
const LAST_RESORT_RENEW_SECS: u64 = 7 * 86_400;

/// What [`publish`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishOutcome {
    pub uploaded: u32,
    pub last_resort_renewed: bool,
    pub available: u32,
    pub last_resort_until: Option<u64>,
}

/// Tops up this device's KeyPackage pool for `persona` to `target`, and
/// renews its last-resort package if it's missing or nearly expired.
pub async fn publish(
    store: &Store,
    client: &Client,
    persona: &Persona,
    target: u32,
) -> Result<PublishOutcome, CoreError> {
    let device = load_device(store, persona.id)?.ok_or_else(|| {
        keys_err(format!(
            "this device has no key for @{}; run `quill keys init --as {}` first",
            persona.handle, persona.handle
        ))
    })?;
    let id = device.id.ok_or_else(|| {
        keys_err(format!(
            "this device isn't registered for @{} yet; run `quill keys init --as {}`",
            persona.handle, persona.handle
        ))
    })?;
    let status = client.key_package_status(persona.id, id).await?;
    let now = now_ms() / 1000;
    let one_time = target.saturating_sub(status.available);
    let renew = status
        .last_resort_until
        .is_none_or(|until| until.saturating_sub(now) < LAST_RESORT_RENEW_SECS);
    if one_time == 0 && !renew {
        return Ok(PublishOutcome {
            uploaded: 0,
            last_resort_renewed: false,
            available: status.available,
            last_resort_until: status.last_resort_until,
        });
    }
    let count = usize::try_from(one_time).unwrap_or(0);
    let packages =
        crate::mls::make_key_packages(store, &persona.actor_uri, &device, count, false, now)?;
    let last_resort = if renew {
        crate::mls::make_key_packages(store, &persona.actor_uri, &device, 1, true, now)?.pop()
    } else {
        None
    };
    let status = client
        .upload_key_packages(
            persona.id,
            id,
            &quill_proto::keys::UploadKeyPackages {
                key_packages: packages.iter().map(|p| b64(p)).collect(),
                last_resort: last_resort.as_deref().map(b64),
            },
        )
        .await?;
    Ok(PublishOutcome {
        uploaded: one_time,
        last_resort_renewed: renew,
        available: status.available,
        last_resort_until: status.last_resort_until,
    })
}

/// A claimed KeyPackage that passed every check.
#[derive(Debug)]
pub struct ClaimedPackage {
    pub device: Uuid,
    pub key_package: openmls::prelude::KeyPackage,
    pub last_resort: bool,
}

/// The result of claiming a persona's KeyPackages.
#[derive(Debug, Default)]
pub struct Claim {
    pub packages: Vec<ClaimedPackage>,
    /// Packages handed out but refused, with why. Never to be used.
    pub refused: Vec<(Uuid, String)>,
    /// Valid devices the server returned no package for.
    pub missing: Vec<Uuid>,
}

/// Claims one KeyPackage per device of the persona at `actor_uri` and
/// checks each against the directory and the pinned identity key. A
/// changed identity key stops the claim until the user accepts it.
pub async fn claim(
    store: &Store,
    client: &Client,
    handle: &str,
    actor_uri: &str,
) -> Result<Claim, CoreError> {
    claim_for(store, client, handle, actor_uri, None).await
}

/// As [`claim`], for one device only when `only` is set.
pub async fn claim_for(
    store: &Store,
    client: &Client,
    handle: &str,
    actor_uri: &str,
    only: Option<Uuid>,
) -> Result<Claim, CoreError> {
    let now = now_ms();
    let published = client.actor_keys(handle).await?;
    let checked = check_published(&published, actor_uri, now)?;
    let (identity_key, version) = checked
        .identity
        .clone()
        .ok_or_else(|| keys_err(format!("@{handle} has no keys yet")))?;
    if let PinStatus::Changed { .. } = check_pin(store, actor_uri, &identity_key, version)? {
        return Err(keys_err(format!(
            "@{handle}'s identity key changed since it was pinned; check it with `quill keys show {handle}` first"
        )));
    }
    let claimed = client.claim_key_packages_for(handle, only).await?;
    if claimed.actor_uri != actor_uri {
        return Err(keys_err("the server returned packages for another persona"));
    }
    let mut result = Claim::default();
    for p in &claimed.packages {
        let Some(device) = checked.devices.iter().find(|d| d.id == p.device) else {
            result
                .refused
                .push((p.device, "not a device the directory lists".into()));
            continue;
        };
        if !matches!(device.status, DeviceStatus::Valid { .. }) {
            result
                .refused
                .push((p.device, format!("its device is {:?}", device.status)));
            continue;
        }
        let checked_package = unb64(&p.key_package)
            .ok_or_else(|| keys_err("not base64url"))
            .and_then(|bytes| {
                crate::mls::check_key_package(
                    &bytes,
                    &crate::mls::PackageExpect {
                        actor_uri,
                        identity_key: &identity_key,
                        identity_key_version: version,
                        device_key: &device.device_key,
                        now_ms: now,
                    },
                )
            });
        match checked_package {
            Ok(key_package) => result.packages.push(ClaimedPackage {
                device: p.device,
                key_package,
                last_resort: p.last_resort,
            }),
            Err(e) => result.refused.push((p.device, e.to_string())),
        }
    }
    result.missing = checked
        .devices
        .iter()
        .filter(|d| matches!(d.status, DeviceStatus::Valid { .. }))
        .filter(|d| only.is_none_or(|o| o == d.id))
        .filter(|d| !claimed.packages.iter().any(|p| p.device == d.id))
        .map(|d| d.id)
        .collect();
    Ok(result)
}

// ---- Checking published keys ------------------------------------------------

/// A published device, as this client judges it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceStatus {
    /// Certified by the current identity key.
    Valid { limited: bool },
    /// Certified by an older identity key; not usable until certified again.
    Superseded,
    /// Revoked by the identity key.
    Revoked(RevocationReason),
    /// Fails verification: a forgery, or damage. Never use it.
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedDevice {
    pub id: Uuid,
    pub device_key: Vec<u8>,
    pub status: DeviceStatus,
}

/// A persona's published keys after checking every certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedKeys {
    pub actor_uri: String,
    pub identity: Option<(Vec<u8>, u32)>,
    pub devices: Vec<CheckedDevice>,
}

impl CheckedKeys {
    pub fn fingerprint(&self) -> Option<String> {
        self.identity.as_ref().map(|(k, _)| fingerprint(k))
    }
}

/// Checks a persona's published keys: that they belong to `actor_uri`, and
/// every device's certificate and revocation against the identity key.
pub fn check_published(
    keys: &PersonaKeys,
    actor_uri: &str,
    now: u64,
) -> Result<CheckedKeys, CoreError> {
    if keys.actor_uri != actor_uri {
        return Err(keys_err(format!(
            "the server sent keys for {} when asked for {actor_uri}",
            keys.actor_uri
        )));
    }
    let identity = published_identity(keys)?;
    if let Some((k, _)) = &identity {
        quill_certs::public_key(k)?;
    }
    let devices = keys
        .devices
        .iter()
        .map(|d| {
            let device_key = unb64(&d.device_key).unwrap_or_default();
            let status = device_status(d, &device_key, identity.as_ref(), actor_uri, now);
            CheckedDevice {
                id: d.id,
                device_key,
                status,
            }
        })
        .collect();
    Ok(CheckedKeys {
        actor_uri: actor_uri.to_owned(),
        identity,
        devices,
    })
}

fn device_status(
    d: &DeviceInfo,
    device_key: &[u8],
    identity: Option<&(Vec<u8>, u32)>,
    actor_uri: &str,
    now: u64,
) -> DeviceStatus {
    let Some((identity_key, version)) = identity else {
        return DeviceStatus::Invalid("the persona has no identity key".into());
    };
    if let Some(r) = &d.revocation {
        let verified = unb64(r)
            .ok_or(quill_certs::CertError::Malformed("not base64url".into()))
            .and_then(|r| DeviceRevocation::verify(&r, actor_uri, identity_key));
        return match verified {
            Ok(rev) if rev.device_key.as_slice() == device_key => DeviceStatus::Revoked(rev.reason),
            Ok(_) => DeviceStatus::Invalid("its revocation is for another device".into()),
            // Signed by an earlier identity key: the device is unusable anyway.
            Err(_) if d.key_version < *version => DeviceStatus::Superseded,
            Err(e) => DeviceStatus::Invalid(format!("its revocation doesn't verify: {e}")),
        };
    }
    if d.key_version < *version {
        return DeviceStatus::Superseded;
    }
    let Some(cert) = unb64(&d.certificate) else {
        return DeviceStatus::Invalid("its certificate is not base64url".into());
    };
    match DeviceCertificate::verify(
        &cert,
        &Expected {
            actor_uri,
            identity_key,
            identity_key_version: *version,
            device_key: Some(device_key),
            now_ms: now,
        },
    ) {
        Ok(c) if c.limited == d.limited => DeviceStatus::Valid { limited: c.limited },
        Ok(_) => DeviceStatus::Invalid("the server misreports whether it's limited".into()),
        Err(e) => DeviceStatus::Invalid(format!("its certificate doesn't verify: {e}")),
    }
}

// ---- Trust on first use ---------------------------------------------------------

/// How a persona's identity key compares with the one first seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinStatus {
    /// Never seen before; now pinned.
    New,
    /// The pinned key.
    Matches,
    /// Different from the pinned key. Not accepted until the user says so.
    Changed {
        pinned_fingerprint: String,
        pinned_version: u32,
    },
}

fn pin(store: &Store, actor_uri: &str, key: &[u8], version: u32) -> Result<(), CoreError> {
    let mut value = version.to_be_bytes().to_vec();
    value.extend_from_slice(key);
    Ok(store.apply(&[Op::Put {
        table: Table::Pins,
        key: actor_uri.as_bytes(),
        value: &value,
    }])?)
}

/// Compares `key` with the pin for `actor_uri`, pinning it if there's none.
pub fn check_pin(
    store: &Store,
    actor_uri: &str,
    key: &[u8],
    version: u32,
) -> Result<PinStatus, CoreError> {
    match store.get(Table::Pins, actor_uri.as_bytes())? {
        None => {
            pin(store, actor_uri, key, version)?;
            Ok(PinStatus::New)
        }
        Some(v) if v.len() == 36 && &v[4..] == key => Ok(PinStatus::Matches),
        Some(v) if v.len() == 36 => {
            let mut ver = [0u8; 4];
            ver.copy_from_slice(&v[..4]);
            Ok(PinStatus::Changed {
                pinned_fingerprint: fingerprint(&v[4..]),
                pinned_version: u32::from_be_bytes(ver),
            })
        }
        Some(_) => Err(keys_err("a stored pin is damaged")),
    }
}

/// Accepts a changed identity key, after the user has checked it.
pub fn accept_pin(
    store: &Store,
    actor_uri: &str,
    key: &[u8],
    version: u32,
) -> Result<(), CoreError> {
    pin(store, actor_uri, key, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_keys_round_trip() {
        let store = Store::in_memory();
        let persona = Uuid::now_v7();
        assert!(load_identity(&store, persona).unwrap().is_none());
        let id = Identity {
            version: 3,
            key: generate().unwrap(),
        };
        save_identity(&store, persona, &id).unwrap();
        let back = load_identity(&store, persona).unwrap().unwrap();
        assert_eq!((back.version, back.public()), (3, id.public()));

        let dev = DeviceKey {
            id: Some(Uuid::new_v4()),
            key: generate().unwrap(),
            certificate: vec![1, 2, 3],
        };
        save_device(&store, persona, &dev).unwrap();
        let back = load_device(&store, persona).unwrap().unwrap();
        assert_eq!(
            (back.id, back.public(), back.certificate),
            (dev.id, dev.public(), vec![1, 2, 3])
        );
        forget_device(&store, persona).unwrap();
        assert!(load_device(&store, persona).unwrap().is_none());
    }

    #[test]
    fn pins_report_changes_without_accepting_them() {
        let store = Store::in_memory();
        let uri = "https://w.example/users/fox";
        assert_eq!(check_pin(&store, uri, &[1; 32], 1).unwrap(), PinStatus::New);
        assert_eq!(
            check_pin(&store, uri, &[1; 32], 1).unwrap(),
            PinStatus::Matches
        );
        let changed = check_pin(&store, uri, &[2; 32], 2).unwrap();
        assert!(matches!(
            changed,
            PinStatus::Changed {
                pinned_version: 1,
                ..
            }
        ));
        // Still changed until accepted.
        assert!(matches!(
            check_pin(&store, uri, &[2; 32], 2).unwrap(),
            PinStatus::Changed { .. }
        ));
        accept_pin(&store, uri, &[2; 32], 2).unwrap();
        assert_eq!(
            check_pin(&store, uri, &[2; 32], 2).unwrap(),
            PinStatus::Matches
        );
    }

    #[test]
    fn certificates_start_at_a_day_boundary() {
        assert_eq!(day_start(DAY_MS * 5 + 12_345), DAY_MS * 5);
        assert_eq!(day_start(DAY_MS * 5), DAY_MS * 5);
    }
}
