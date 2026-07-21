use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
#[cfg(not(windows))]
use directories::ProjectDirs;
use serde::{Deserialize, Deserializer, Serialize};
use tempfile::NamedTempFile;
use yggdrasil::config::Config as YggConfig;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    SystemProxy,
    Proxy,
    Tun,
}

impl Default for ConnectionMode {
    fn default() -> Self {
        if cfg!(feature = "tun") && !is_flatpak() {
            Self::Tun
        } else {
            Self::SystemProxy
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GuiConfig {
    pub mode: ConnectionMode,
    pub proxy_listen: String,
    pub dns_server: String,
    pub dns_port: u16,
}

impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            mode: ConnectionMode::default(),
            proxy_listen: "127.0.0.1:1080".to_owned(),
            dns_server: String::new(),
            dns_port: 53,
        }
    }
}

impl GuiConfig {
    pub fn effective_mode(&self) -> ConnectionMode {
        match self.mode {
            ConnectionMode::Tun if !cfg!(feature = "tun") => ConnectionMode::SystemProxy,
            mode => mode,
        }
    }
}

#[derive(Default, Deserialize)]
struct GuiConfigWire {
    mode: Option<ConnectionMode>,
    // Drosophila 2.0 used this boolean for the userspace SOCKS mode.
    proxy_enabled: Option<bool>,
    proxy_listen: Option<String>,
    dns_server: Option<String>,
    dns_port: Option<u16>,
}

impl<'de> Deserialize<'de> for GuiConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = GuiConfigWire::deserialize(deserializer)?;
        let defaults = Self::default();
        let mode = wire.mode.or_else(|| {
            wire.proxy_enabled.map(|enabled| {
                if enabled {
                    ConnectionMode::Proxy
                } else {
                    ConnectionMode::Tun
                }
            })
        });
        Ok(Self {
            mode: mode.unwrap_or(defaults.mode),
            proxy_listen: wire.proxy_listen.unwrap_or(defaults.proxy_listen),
            dns_server: wire.dns_server.unwrap_or(defaults.dns_server),
            dns_port: wire.dns_port.unwrap_or(defaults.dns_port),
        })
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
        "none".clone_into(&mut yggdrasil.admin_listen);
        "auto".clone_into(&mut yggdrasil.if_name);
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
        Ok(directory.join("yggdrasil.toml"))
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
#[path = "tests/config.rs"]
mod tests;
