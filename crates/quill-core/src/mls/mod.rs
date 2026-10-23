//! MLS on the client (2c): the OpenMLS provider over the encrypted store,
//! KeyPackages carrying device certificates, and checking claimed ones.

pub mod storage;

use ed25519_dalek::SigningKey;
use openmls::prelude::{
    BasicCredential, Capabilities, Ciphersuite, CredentialType, CredentialWithKey, Extension,
    ExtensionType, Extensions, KeyPackage, KeyPackageIn, Lifetime, MlsMessageBodyIn, MlsMessageIn,
    MlsMessageOut, ProtocolVersion, UnknownExtension,
};
use openmls_rust_crypto::RustCrypto;
use openmls_traits::{
    OpenMlsProvider,
    signatures::{Signer, SignerError},
    types::SignatureScheme,
};
use quill_certs::{DeviceCertificate, Expected};
use storage::MlsStorage;
use tls_codec::{Deserialize as _, Serialize as _};

use crate::{CoreError, Store, keys::DeviceKey};

/// The one ciphersuite Quill uses (RFC 9420's mandatory-to-implement).
pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;
/// Leaf extension carrying the device certificate (protocol draft, 6).
pub const DEVICE_CERT_EXTENSION: u16 = 0xF0A1;
/// KeyPackages last this many days after the day they're made.
pub const KEY_PACKAGE_DAYS: u64 = 28;
const DAY_SECS: u64 = 86_400;

fn mls_err(what: &str, e: impl std::fmt::Display) -> CoreError {
    CoreError::Keys(format!("{what}: {e}"))
}

/// OpenMLS's provider: RustCrypto, and storage in the encrypted store.
pub struct Provider<'a> {
    crypto: RustCrypto,
    storage: MlsStorage<'a>,
}

impl<'a> Provider<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self {
            crypto: RustCrypto::default(),
            storage: MlsStorage::new(store),
        }
    }

    /// Runs `f` with all its MLS state writes applied in one batch, or none
    /// if it fails.
    pub fn atomically<T>(
        &self,
        f: impl FnOnce(&Self) -> Result<T, CoreError>,
    ) -> Result<T, CoreError> {
        self.storage.begin().map_err(|e| mls_err("MLS state", e))?;
        match f(self) {
            Ok(v) => {
                self.storage.commit().map_err(|e| mls_err("MLS state", e))?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.storage.rollback();
                Err(e)
            }
        }
    }
}

impl<'a> OpenMlsProvider for Provider<'a> {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = MlsStorage<'a>;

    fn storage(&self) -> &Self::StorageProvider {
        &self.storage
    }

    fn crypto(&self) -> &Self::CryptoProvider {
        &self.crypto
    }

    fn rand(&self) -> &Self::RandProvider {
        &self.crypto
    }
}

/// Signs as this device: its Ed25519 key is its MLS signature key.
pub(crate) struct DeviceSigner<'k>(pub &'k SigningKey);

impl Signer for DeviceSigner<'_> {
    fn sign(&self, payload: &[u8]) -> Result<Vec<u8>, SignerError> {
        Ok(ed25519_dalek::Signer::sign(self.0, payload)
            .to_bytes()
            .to_vec())
    }

    fn signature_scheme(&self) -> SignatureScheme {
        SignatureScheme::ED25519
    }
}

fn credential(actor_uri: &str, device: &DeviceKey) -> CredentialWithKey {
    CredentialWithKey {
        credential: BasicCredential::new(actor_uri.as_bytes().to_vec()).into(),
        signature_key: device.public().to_vec().into(),
    }
}

/// The capabilities every Quill leaf advertises: our ciphersuite, basic
/// credentials, the device-certificate extension, and the last-resort
/// KeyPackage extension (which a package can only carry if listed).
pub fn capabilities() -> Capabilities {
    Capabilities::new(
        None,
        Some(&[CIPHERSUITE]),
        Some(&[
            ExtensionType::Unknown(DEVICE_CERT_EXTENSION),
            ExtensionType::LastResort,
        ]),
        None,
        Some(&[CredentialType::Basic]),
    )
}

/// A KeyPackage lifetime: from the day before the UTC day of `now` to
/// [`KEY_PACKAGE_DAYS`] days after it, in seconds.
pub fn lifetime(now_secs: u64) -> (u64, u64) {
    let day = now_secs - now_secs % DAY_SECS;
    (day - DAY_SECS, day + KEY_PACKAGE_DAYS * DAY_SECS)
}

/// Makes KeyPackages for this device acting as `actor_uri`, as encoded
/// `MLSMessage`s. Their private keys are saved in the store, all at once.
pub fn make_key_packages(
    store: &Store,
    actor_uri: &str,
    device: &DeviceKey,
    count: usize,
    last_resort: bool,
    now_secs: u64,
) -> Result<Vec<Vec<u8>>, CoreError> {
    let provider = Provider::new(store);
    let (not_before, not_after) = lifetime(now_secs);
    let extensions = Extensions::single(Extension::Unknown(
        DEVICE_CERT_EXTENSION,
        UnknownExtension(device.certificate.clone()),
    ))
    .map_err(|e| mls_err("leaf extension", e))?;
    provider.atomically(|provider| {
        (0..count)
            .map(|_| {
                let mut builder = KeyPackage::builder()
                    .key_package_lifetime(Lifetime::init(not_before, not_after))
                    .leaf_node_capabilities(capabilities())
                    .leaf_node_extensions(extensions.clone());
                if last_resort {
                    builder = builder.mark_as_last_resort();
                }
                let bundle = builder
                    .build(
                        CIPHERSUITE,
                        provider,
                        &DeviceSigner(device.signing_key()),
                        credential(actor_uri, device),
                    )
                    .map_err(|e| mls_err("making a KeyPackage", e))?;
                MlsMessageOut::from(bundle.key_package().clone())
                    .tls_serialize_detached()
                    .map_err(|e| mls_err("encoding a KeyPackage", e))
            })
            .collect()
    })
}

/// What a claimed KeyPackage must match.
#[derive(Debug, Clone, Copy)]
pub struct PackageExpect<'a> {
    pub actor_uri: &'a str,
    pub identity_key: &'a [u8],
    pub identity_key_version: u32,
    /// The device's key as the directory lists it.
    pub device_key: &'a [u8],
    pub now_ms: u64,
}

/// Checks a claimed KeyPackage: OpenMLS's own validation (signatures,
/// lifetime, ...), then that it's ours: the ciphersuite, a basic credential
/// naming the persona, this device's signature key, our capabilities, and
/// a device certificate that verifies under the persona's identity key.
pub fn check_key_package(bytes: &[u8], e: &PackageExpect<'_>) -> Result<KeyPackage, CoreError> {
    let refuse = |why: String| CoreError::Keys(format!("KeyPackage refused: {why}"));
    let message =
        MlsMessageIn::tls_deserialize_exact(bytes).map_err(|err| refuse(err.to_string()))?;
    let MlsMessageBodyIn::KeyPackage(package) = message.extract() else {
        return Err(refuse("not a KeyPackage".into()));
    };
    let package: KeyPackageIn = package;
    let package = package
        .validate(&RustCrypto::default(), ProtocolVersion::Mls10)
        .map_err(|err| refuse(err.to_string()))?;
    if package.ciphersuite() != CIPHERSUITE {
        return Err(refuse("unexpected ciphersuite".into()));
    }
    let leaf = package.leaf_node();
    let credential = BasicCredential::try_from(leaf.credential().clone())
        .map_err(|_| refuse("not a basic credential".into()))?;
    if credential.identity() != e.actor_uri.as_bytes() {
        return Err(refuse("its credential names another actor".into()));
    }
    if leaf.signature_key().as_slice() != e.device_key {
        return Err(refuse("it isn't signed by this device's key".into()));
    }
    if !leaf
        .capabilities()
        .extensions()
        .contains(&ExtensionType::Unknown(DEVICE_CERT_EXTENSION))
    {
        return Err(refuse(
            "it doesn't list the device-certificate extension".into(),
        ));
    }
    let cert = leaf
        .extensions()
        .unknown(DEVICE_CERT_EXTENSION)
        .ok_or_else(|| refuse("it has no device certificate".into()))?;
    DeviceCertificate::verify(
        &cert.0,
        &Expected {
            actor_uri: e.actor_uri,
            identity_key: e.identity_key,
            identity_key_version: e.identity_key_version,
            device_key: Some(e.device_key),
            now_ms: e.now_ms,
        },
    )
    .map_err(|err| refuse(format!("its device certificate: {err}")))?;
    Ok(package)
}

#[cfg(test)]
mod tests;
