//! The app's own settings file: the servers it knows and how it looks.
//! Nothing secret lives here; each server's token and keys are in its own
//! encrypted store.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Overrides where the app keeps its settings and stores (tests, and
/// running two copies side by side).
pub const DIR_ENV: &str = "QUILL_GUI_DIR";

/// Where the settings file and the default stores folder are.
#[derive(Debug, Clone)]
pub struct Paths {
    pub settings: PathBuf,
    pub default_stores: PathBuf,
}

impl Paths {
    /// `$QUILL_GUI_DIR`, else the platform's config and data folders.
    pub fn default_paths() -> anyhow::Result<Self> {
        if let Some(dir) = std::env::var_os(DIR_ENV) {
            return Ok(Self::in_dir(Path::new(&dir)));
        }
        let dirs = directories::ProjectDirs::from("", "", "quill-gui")
            .ok_or_else(|| anyhow::anyhow!("no home folder; set {DIR_ENV}"))?;
        Ok(Self {
            settings: dirs.config_dir().join("settings.json"),
            default_stores: dirs.data_dir().join("stores"),
        })
    }

    /// Everything under one folder.
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            settings: dir.join("settings.json"),
            default_stores: dir.join("stores"),
        }
    }
}

/// One server, as the settings file keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub id: Uuid,
    pub address: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

fn default_text_size() -> u16 {
    100
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub servers: Vec<Server>,
    /// The server open when the app last closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Uuid>,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "default_text_size")]
    pub text_size: u16,
    #[serde(default)]
    pub advanced: bool,
    /// Where the stores are, when not in the default folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stores_folder: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            current: None,
            theme: Theme::System,
            text_size: default_text_size(),
            advanced: false,
            stores_folder: None,
        }
    }
}

impl Settings {
    /// Reads the file; a missing file is a fresh install.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes the file whole, through a temporary file so a crash never
    /// leaves half of it.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn server(&self, id: Uuid) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    pub fn server_mut(&mut self, id: Uuid) -> Option<&mut Server> {
        self.servers.iter_mut().find(|s| s.id == id)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn settings_round_trip_and_start_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        let mut s = Settings::default();
        let id = Uuid::new_v4();
        s.servers.push(Server {
            id,
            address: "https://writers.example".into(),
            name: "writers.example".into(),
            login: Some("ink".into()),
        });
        s.current = Some(id);
        s.advanced = true;
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        // Older files without newer fields still load.
        std::fs::write(&path, br#"{"servers": []}"#).unwrap();
        assert_eq!(Settings::load(&path).unwrap().text_size, 100);
    }
}
