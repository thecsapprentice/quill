//! The local profile: which server this client talks to and its login token.
//!
//! Phase 0 stores the token in a file readable only by the current user.
//! Phase 2 moves it, along with all key material, into the encrypted local
//! store (SQLCipher, keyed from the OS keychain).

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::CoreError;

/// Environment variable that overrides the profile file location.
pub const PROFILE_ENV: &str = "QUILL_PROFILE";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// Server base URL, e.g. `https://writers.example`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// Bearer token for the logged-in account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Profile")
            .field("server", &self.server)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Profile {
    /// Default profile path: `$QUILL_PROFILE`, else the platform config
    /// directory (e.g. `~/.config/quill/profile.json` on Linux).
    pub fn default_path() -> Result<PathBuf, CoreError> {
        if let Some(p) = std::env::var_os(PROFILE_ENV) {
            return Ok(PathBuf::from(p));
        }
        let dirs = directories::ProjectDirs::from("", "", "quill").ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no home directory; set QUILL_PROFILE",
            )
        })?;
        Ok(dirs.config_dir().join("profile.json"))
    }

    /// Loads a profile, returning an empty one if the file does not exist.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes the profile atomically, with owner-only permissions on Unix.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let body = Zeroizing::new(serde_json::to_vec_pretty(self)?);
        {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&tmp)?;
            file.write_all(&body)?;
            file.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("profile.json");

        assert!(Profile::load(&path).unwrap().server.is_none());

        let profile = Profile {
            server: Some("https://writers.example".into()),
            token: Some(Zeroizing::new("qt_secret".into())),
        };
        profile.save(&path).unwrap();
        let loaded = Profile::load(&path).unwrap();
        assert_eq!(loaded.server.as_deref(), Some("https://writers.example"));
        assert_eq!(
            loaded.token.as_deref().map(String::as_str),
            Some("qt_secret")
        );
        assert!(!format!("{loaded:?}").contains("qt_secret"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
