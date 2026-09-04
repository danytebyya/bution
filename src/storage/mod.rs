//! Persistent node settings and trust store.

use crate::cluster::NodeRole;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub settings_file: PathBuf,
    pub identity_file: PathBuf,
    pub noise_identity_file: PathBuf,
    pub cache_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self> {
        let project = ProjectDirs::from("dev", "BUTION", "BUTION")
            .context("could not determine the application data directory")?;
        let data_dir = project.data_local_dir().to_path_buf();
        Ok(Self {
            settings_file: data_dir.join("settings.toml"),
            identity_file: data_dir.join("identity.key"),
            noise_identity_file: data_dir.join("noise-identity.key"),
            cache_dir: project.cache_dir().to_path_buf(),
            data_dir,
        })
    }

    #[cfg(test)]
    fn for_test(data_dir: PathBuf) -> Self {
        Self {
            settings_file: data_dir.join("settings.toml"),
            identity_file: data_dir.join("identity.key"),
            noise_identity_file: data_dir.join("noise-identity.key"),
            cache_dir: data_dir.join("cache"),
            data_dir,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        fs::create_dir_all(&self.data_dir)
            .context("could not create application data directory")?;
        fs::create_dir_all(&self.cache_dir).context("could not create cache directory")?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub id: Uuid,
    pub name: String,
    pub public_key: String,
    pub paired_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub node_id: Uuid,
    pub node_name: String,
    pub role: NodeRole,
    pub control_port: u16,
    pub rpc_port: u16,
    pub llama_bin_dir: Option<PathBuf>,
    pub last_model: Option<PathBuf>,
    pub trusted_peers: Vec<TrustedPeer>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            node_id: Uuid::new_v4(),
            node_name: hostname::get()
                .ok()
                .and_then(|name| name.into_string().ok())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "BUTION Node".into()),
            role: NodeRole::Automatic,
            control_port: 31_750,
            rpc_port: 50_052,
            llama_bin_dir: None,
            last_model: None,
            trusted_peers: Vec::new(),
        }
    }
}

impl Settings {
    pub fn load_or_create(paths: &AppPaths) -> Result<Self> {
        paths.ensure()?;
        if !paths.settings_file.exists() {
            let settings = Self::default();
            settings.save(paths)?;
            return Ok(settings);
        }

        let text = fs::read_to_string(&paths.settings_file)
            .with_context(|| format!("could not read {}", paths.settings_file.display()))?;
        let settings: Self = toml::from_str(&text).context("settings file is not valid TOML")?;
        if settings.node_id.is_nil() {
            bail!("settings contain an invalid nil node UUID");
        }
        Ok(settings)
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        paths.ensure()?;
        let text = toml::to_string_pretty(self).context("could not serialize settings")?;
        atomic_write(&paths.settings_file, text.as_bytes())
    }

    pub fn trust(&mut self, peer: TrustedPeer) {
        self.trusted_peers.retain(|existing| existing.id != peer.id);
        self.trusted_peers.push(peer);
    }

    pub fn trusted_peer(&self, id: Uuid) -> Option<&TrustedPeer> {
        self.trusted_peers.iter().find(|peer| peer.id == id)
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("could not replace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_paths() -> AppPaths {
        AppPaths::for_test(std::env::temp_dir().join(format!("bution-test-{}", Uuid::new_v4())))
    }

    #[test]
    fn settings_keep_permanent_node_id() {
        let paths = temporary_paths();
        let first = Settings::load_or_create(&paths).unwrap();
        let second = Settings::load_or_create(&paths).unwrap();
        assert_eq!(first.node_id, second.node_id);
        fs::remove_dir_all(paths.data_dir).unwrap();
    }

    #[test]
    fn trust_store_replaces_rotated_peer_key() {
        let mut settings = Settings::default();
        let id = Uuid::new_v4();
        for key in ["old", "new"] {
            settings.trust(TrustedPeer {
                id,
                name: "worker".into(),
                public_key: key.into(),
                paired_at: Utc::now(),
            });
        }
        assert_eq!(settings.trusted_peers.len(), 1);
        assert_eq!(settings.trusted_peer(id).unwrap().public_key, "new");
    }
}
