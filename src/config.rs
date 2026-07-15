use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(not(windows))]
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use yggdrasil::config::Config as YggConfig;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiConfig {
    pub proxy_enabled: bool,
    pub proxy_listen: String,
    pub dns_server: String,
    pub dns_port: u16,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            // A sandboxed Flatpak cannot acquire CAP_NET_ADMIN for a host TUN.
            // Start new Flatpak installs in the fully userspace mode instead.
            proxy_enabled: is_flatpak(),
            proxy_listen: "127.0.0.1:1080".to_owned(),
            dns_server: String::new(),
            dns_port: 53,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredConfig {
    #[serde(flatten)]
    pub yggdrasil: YggConfig,
    #[serde(default)]
    pub drosophila: GuiConfig,
}

impl Default for StoredConfig {
    fn default() -> Self {
        let mut yggdrasil = YggConfig::generate();
        // The GUI talks to Core directly. Exposing an admin TCP socket would be
        // redundant and would broaden the local attack surface.
        yggdrasil.admin_listen = "none".to_owned();
        yggdrasil.if_name = "auto".to_owned();
        Self {
            yggdrasil,
            drosophila: GuiConfig::default(),
        }
    }
}

impl StoredConfig {
    pub fn regenerate_private_key(&mut self) {
        self.yggdrasil.private_key = YggConfig::generate().private_key;
    }
}

pub fn is_flatpak() -> bool {
    Path::new("/.flatpak-info").is_file()
}

pub fn config_path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let executable = std::env::current_exe().context("locating Drosophila.exe")?;
        let directory = executable
            .parent()
            .context("Drosophila.exe has no parent directory")?;
        return Ok(directory.join("yggdrasil.toml"));
    }

    #[cfg(not(windows))]
    {
        let dirs = ProjectDirs::from("io.github", "ergolyam", "Drosophila")
            .context("the operating system did not provide a configuration directory")?;
        Ok(dirs.config_dir().join("yggdrasil.toml"))
    }
}

pub fn load_or_create(path: &Path) -> Result<StoredConfig> {
    if !path.exists() {
        let config = StoredConfig::default();
        save(path, &config)?;
        return Ok(config);
    }

    let text = fs::read_to_string(path)
        .with_context(|| format!("reading configuration {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing configuration {}", path.display()))
}

pub fn save(path: &Path, config: &StoredConfig) -> Result<()> {
    let parent = path
        .parent()
        .context("configuration path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating configuration directory {}", parent.display()))?;

    let text = toml::to_string_pretty(config).context("serializing configuration")?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("creating a temporary file in {}", parent.display()))?;
    temporary
        .write_all(text.as_bytes())
        .context("writing configuration")?;
    temporary.flush().context("flushing configuration")?;
    temporary
        .as_file()
        .sync_all()
        .context("syncing configuration")?;
    temporary
        .persist(path)
        .map_err(|error| io::Error::new(error.error.kind(), error.error))
        .with_context(|| format!("replacing configuration {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_yggdrasil_and_gui_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("yggdrasil.toml");
        let mut expected = StoredConfig::default();
        expected.yggdrasil.peers = vec!["tls://example.com:443".to_owned()];
        expected.drosophila.proxy_enabled = true;

        save(&path, &expected).unwrap();
        let actual = load_or_create(&path).unwrap();

        assert_eq!(actual.yggdrasil.peers, expected.yggdrasil.peers);
        assert!(actual.drosophila.proxy_enabled);
        assert_eq!(actual.yggdrasil.admin_listen, "none");
    }
}
