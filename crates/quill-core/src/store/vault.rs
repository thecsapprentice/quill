//! Where a keychain-protected store keeps its key: the operating system's
//! credential store (Windows Credential Manager, the macOS Keychain, or the
//! Secret Service on Linux desktops).

use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use super::StoreError;

/// Service name the store's keys are filed under.
const SERVICE: &str = "quill-store";

/// A place to keep small secrets by name.
pub trait Vault: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError>;
    fn set(&self, name: &str, secret: &[u8]) -> Result<(), StoreError>;
    /// Removes a secret; succeeds if it's already gone.
    fn delete(&self, name: &str) -> Result<(), StoreError>;
}

fn vault_err(e: keyring_core::Error) -> StoreError {
    StoreError::Keychain(e.to_string())
}

/// The operating system's credential store.
pub struct OsKeychain(Arc<keyring_core::CredentialStore>);

impl OsKeychain {
    /// Connects to this platform's credential store, or explains why not
    /// (for example, a Linux machine with no desktop keyring running).
    pub fn new() -> Result<Self, StoreError> {
        Ok(Self(platform_store()?))
    }

    fn entry(&self, name: &str) -> Result<keyring_core::Entry, StoreError> {
        self.0.build(SERVICE, name, None).map_err(vault_err)
    }
}

#[cfg(windows)]
fn platform_store() -> Result<Arc<keyring_core::CredentialStore>, StoreError> {
    Ok(windows_native_keyring_store::Store::new().map_err(vault_err)?)
}

#[cfg(target_vendor = "apple")]
fn platform_store() -> Result<Arc<keyring_core::CredentialStore>, StoreError> {
    Ok(apple_native_keyring_store::keychain::Store::new().map_err(vault_err)?)
}

#[cfg(target_os = "linux")]
fn platform_store() -> Result<Arc<keyring_core::CredentialStore>, StoreError> {
    Ok(zbus_secret_service_keyring_store::Store::new().map_err(vault_err)?)
}

#[cfg(not(any(windows, target_vendor = "apple", target_os = "linux")))]
fn platform_store() -> Result<Arc<keyring_core::CredentialStore>, StoreError> {
    Err(StoreError::Keychain(
        "no supported keychain on this platform".into(),
    ))
}

impl Vault for OsKeychain {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
        match self.entry(name)?.get_secret() {
            Ok(s) => Ok(Some(Zeroizing::new(s))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(vault_err(e)),
        }
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<(), StoreError> {
        self.entry(name)?.set_secret(secret).map_err(vault_err)
    }

    fn delete(&self, name: &str) -> Result<(), StoreError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(vault_err(e)),
        }
    }
}

type Secrets = std::collections::HashMap<String, Zeroizing<Vec<u8>>>;

/// An in-memory vault, for tests.
#[derive(Default)]
pub struct MemoryVault(Mutex<Secrets>);

impl MemoryVault {
    fn map(&self) -> Result<std::sync::MutexGuard<'_, Secrets>, StoreError> {
        self.0
            .lock()
            .map_err(|_| StoreError::Keychain("memory vault poisoned".into()))
    }

    /// How many secrets it holds.
    pub fn len(&self) -> usize {
        self.map().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Vault for MemoryVault {
    fn get(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
        Ok(self.map()?.get(name).cloned())
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<(), StoreError> {
        self.map()?
            .insert(name.to_owned(), Zeroizing::new(secret.to_vec()));
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), StoreError> {
        self.map()?.remove(name);
        Ok(())
    }
}
