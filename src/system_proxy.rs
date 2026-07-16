use std::fs;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::config::config_path;

const STATE_VERSION: u32 = 1;
const STATE_FILE: &str = "system-proxy-state.json";

#[derive(Deserialize, Serialize)]
struct RecoveryState {
    version: u32,
    original: platform::Snapshot,
    expected: platform::Snapshot,
}

pub(crate) struct SystemProxy {
    path: PathBuf,
    state: RecoveryState,
    active: bool,
}

impl SystemProxy {
    pub(crate) fn enable(listen: SocketAddr) -> Result<Self> {
        ensure!(
            listen.ip().is_loopback(),
            "the system proxy must listen on a loopback address"
        );
        recover_stale()?;

        let path = state_path()?;
        let original = platform::snapshot().context("reading the current system proxy")?;
        let expected = platform::local_proxy(listen, &original);
        let state = RecoveryState {
            version: STATE_VERSION,
            original,
            expected,
        };
        write_state(&path, &state)?;

        if let Err(error) = platform::apply(&state.expected) {
            let _ = platform::apply(&state.original);
            let _ = remove_state(&path);
            return Err(error).context("enabling the system proxy");
        }
        let current = platform::snapshot().context("verifying the system proxy")?;
        if !platform::is_owned(&current, &state.expected) {
            let _ = platform::apply(&state.original);
            let _ = remove_state(&path);
            bail!("the desktop rejected the system proxy settings");
        }

        Ok(Self {
            path,
            state,
            active: true,
        })
    }

    pub(crate) fn close(mut self) {
        self.restore();
    }

    fn restore(&mut self) {
        if !self.active {
            return;
        }
        match restore_if_owned(&self.path, &self.state) {
            Ok(()) => self.active = false,
            Err(error) => tracing::error!(%error, "failed to restore the system proxy"),
        }
    }
}

impl Drop for SystemProxy {
    fn drop(&mut self) {
        self.restore();
    }
}

pub(crate) fn recover_stale() -> Result<()> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(());
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("reading stale proxy state {}", path.display()))?;
    let state: RecoveryState = serde_json::from_str(&text)
        .with_context(|| format!("parsing stale proxy state {}", path.display()))?;
    ensure!(
        state.version == STATE_VERSION,
        "unsupported system proxy state version {}",
        state.version
    );
    restore_if_owned(&path, &state)
}

fn restore_if_owned(path: &Path, state: &RecoveryState) -> Result<()> {
    let current = platform::snapshot().context("reading the system proxy before restoration")?;
    if platform::is_owned(&current, &state.expected) {
        platform::apply(&state.original).context("restoring the previous system proxy")?;
    } else {
        tracing::warn!("system proxy settings changed outside Drosophila; leaving them untouched");
    }
    remove_state(path)
}

fn state_path() -> Result<PathBuf> {
    let config = config_path()?;
    Ok(config.with_file_name(STATE_FILE))
}

fn write_state(path: &Path, state: &RecoveryState) -> Result<()> {
    let parent = path
        .parent()
        .context("system proxy state path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating state directory {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(state).context("serializing system proxy state")?;
    let mut temporary = NamedTempFile::new_in(parent)
        .with_context(|| format!("creating a temporary file in {}", parent.display()))?;
    temporary.write_all(&bytes).context("writing proxy state")?;
    temporary.flush().context("flushing proxy state")?;
    temporary
        .as_file()
        .sync_all()
        .context("syncing proxy state")?;
    temporary
        .persist(path)
        .map_err(|error| io::Error::new(error.error.kind(), error.error))
        .with_context(|| format!("replacing proxy state {}", path.display()))?;
    Ok(())
}

fn remove_state(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("removing proxy state {}", path.display()))
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::net::SocketAddr;

    use anyhow::{Context, Result, ensure};
    use gtk::gio;
    use gtk::gio::prelude::*;
    use serde::{Deserialize, Serialize};

    const ROOT_SCHEMA: &str = "org.gnome.system.proxy";
    const HTTP_SCHEMA: &str = "org.gnome.system.proxy.http";
    const HTTPS_SCHEMA: &str = "org.gnome.system.proxy.https";
    const SOCKS_SCHEMA: &str = "org.gnome.system.proxy.socks";

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    pub(super) struct Snapshot {
        mode: String,
        use_same_proxy: bool,
        ignore_hosts: Vec<String>,
        http_host: String,
        http_port: i32,
        https_host: String,
        https_port: i32,
        socks_host: String,
        socks_port: i32,
    }

    pub(super) fn snapshot() -> Result<Snapshot> {
        let root = settings(ROOT_SCHEMA)?;
        let http = settings(HTTP_SCHEMA)?;
        let https = settings(HTTPS_SCHEMA)?;
        let socks = settings(SOCKS_SCHEMA)?;
        Ok(Snapshot {
            mode: root.string("mode").to_string(),
            use_same_proxy: root.boolean("use-same-proxy"),
            ignore_hosts: root
                .strv("ignore-hosts")
                .iter()
                .map(ToString::to_string)
                .collect(),
            http_host: http.string("host").to_string(),
            http_port: http.int("port"),
            https_host: https.string("host").to_string(),
            https_port: https.int("port"),
            socks_host: socks.string("host").to_string(),
            socks_port: socks.int("port"),
        })
    }

    pub(super) fn local_proxy(listen: SocketAddr, _original: &Snapshot) -> Snapshot {
        let host = listen.ip().to_string();
        let port = i32::from(listen.port());
        Snapshot {
            mode: "manual".to_owned(),
            use_same_proxy: false,
            ignore_hosts: vec![
                "localhost".to_owned(),
                "127.0.0.0/8".to_owned(),
                "::1".to_owned(),
            ],
            http_host: host.clone(),
            http_port: port,
            https_host: host.clone(),
            https_port: port,
            socks_host: host,
            socks_port: port,
        }
    }

    pub(super) fn apply(snapshot: &Snapshot) -> Result<()> {
        let root = settings(ROOT_SCHEMA)?;
        let http = settings(HTTP_SCHEMA)?;
        let https = settings(HTTPS_SCHEMA)?;
        let socks = settings(SOCKS_SCHEMA)?;

        // Keep the desktop from observing a half-written manual configuration.
        root.set_string("mode", "none")?;
        http.set_string("host", &snapshot.http_host)?;
        http.set_int("port", snapshot.http_port)?;
        https.set_string("host", &snapshot.https_host)?;
        https.set_int("port", snapshot.https_port)?;
        socks.set_string("host", &snapshot.socks_host)?;
        socks.set_int("port", snapshot.socks_port)?;
        root.set_boolean("use-same-proxy", snapshot.use_same_proxy)?;
        let ignore_hosts: Vec<&str> = snapshot.ignore_hosts.iter().map(String::as_str).collect();
        root.set_strv("ignore-hosts", ignore_hosts)?;
        root.set_string("mode", &snapshot.mode)?;
        gio::Settings::sync();
        Ok(())
    }

    pub(super) fn is_owned(current: &Snapshot, expected: &Snapshot) -> bool {
        if current == expected {
            return true;
        }
        current.mode == "manual"
            && ((current.http_host == expected.http_host
                && current.http_port == expected.http_port)
                || (current.https_host == expected.https_host
                    && current.https_port == expected.https_port)
                || (current.socks_host == expected.socks_host
                    && current.socks_port == expected.socks_port))
    }

    fn settings(schema: &str) -> Result<gio::Settings> {
        let source = gio::SettingsSchemaSource::default()
            .context("the desktop did not provide a GSettings schema source")?;
        ensure!(
            source.lookup(schema, true).is_some(),
            "the desktop does not provide the {schema} settings schema"
        );
        Ok(gio::Settings::new(schema))
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::net::SocketAddr;
    use std::ptr::{null, null_mut};

    use anyhow::{Context, Result, bail};
    use serde::{Deserialize, Serialize};
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::Networking::WinInet::{
        INTERNET_OPTION_PER_CONNECTION_OPTION, INTERNET_OPTION_REFRESH,
        INTERNET_OPTION_SETTINGS_CHANGED, INTERNET_PER_CONN_AUTOCONFIG_URL,
        INTERNET_PER_CONN_FLAGS, INTERNET_PER_CONN_FLAGS_UI, INTERNET_PER_CONN_OPTION_LISTW,
        INTERNET_PER_CONN_OPTIONW, INTERNET_PER_CONN_OPTIONW_0, INTERNET_PER_CONN_PROXY_BYPASS,
        INTERNET_PER_CONN_PROXY_SERVER, InternetQueryOptionW, InternetSetOptionW,
        PROXY_TYPE_DIRECT, PROXY_TYPE_PROXY,
    };

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    pub(super) struct Snapshot {
        flags: u32,
        proxy_server: Option<String>,
        proxy_bypass: Option<String>,
        autoconfig_url: Option<String>,
    }

    pub(super) fn snapshot() -> Result<Snapshot> {
        let mut options = [
            option(INTERNET_PER_CONN_FLAGS_UI),
            option(INTERNET_PER_CONN_PROXY_SERVER),
            option(INTERNET_PER_CONN_PROXY_BYPASS),
            option(INTERNET_PER_CONN_AUTOCONFIG_URL),
        ];
        let mut list = option_list(&mut options);
        let mut size = u32::try_from(size_of::<INTERNET_PER_CONN_OPTION_LISTW>()).unwrap();
        let success = unsafe {
            InternetQueryOptionW(
                null(),
                INTERNET_OPTION_PER_CONNECTION_OPTION,
                (&raw mut list).cast::<c_void>(),
                &raw mut size,
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error())
                .context("querying the Windows internet options");
        }

        let flags = unsafe { options[0].Value.dwValue };
        let proxy_server = unsafe { take_wide(options[1].Value.pszValue) };
        let proxy_bypass = unsafe { take_wide(options[2].Value.pszValue) };
        let autoconfig_url = unsafe { take_wide(options[3].Value.pszValue) };
        Ok(Snapshot {
            flags,
            proxy_server,
            proxy_bypass,
            autoconfig_url,
        })
    }

    pub(super) fn local_proxy(listen: SocketAddr, original: &Snapshot) -> Snapshot {
        let host = match listen {
            SocketAddr::V4(address) => address.ip().to_string(),
            SocketAddr::V6(address) => format!("[{}]", address.ip()),
        };
        let endpoint = format!("{host}:{}", listen.port());
        Snapshot {
            flags: PROXY_TYPE_DIRECT | PROXY_TYPE_PROXY,
            proxy_server: Some(format!("http={endpoint};https={endpoint};socks={endpoint}")),
            proxy_bypass: Some("<local>;localhost;127.*;[::1]".to_owned()),
            autoconfig_url: original.autoconfig_url.clone(),
        }
    }

    pub(super) fn apply(snapshot: &Snapshot) -> Result<()> {
        let mut proxy_server = snapshot.proxy_server.as_deref().map(wide);
        let mut proxy_bypass = snapshot.proxy_bypass.as_deref().map(wide);
        let mut options = vec![INTERNET_PER_CONN_OPTIONW {
            dwOption: INTERNET_PER_CONN_FLAGS,
            Value: INTERNET_PER_CONN_OPTIONW_0 {
                dwValue: snapshot.flags,
            },
        }];
        if proxy_server.is_some() {
            options.push(string_option(
                INTERNET_PER_CONN_PROXY_SERVER,
                proxy_server.as_mut(),
            ));
        }
        if proxy_bypass.is_some() {
            options.push(string_option(
                INTERNET_PER_CONN_PROXY_BYPASS,
                proxy_bypass.as_mut(),
            ));
        }
        let list = option_list(&mut options);
        let success = unsafe {
            InternetSetOptionW(
                null(),
                INTERNET_OPTION_PER_CONNECTION_OPTION,
                (&raw const list).cast::<c_void>(),
                u32::try_from(size_of::<INTERNET_PER_CONN_OPTION_LISTW>()).unwrap(),
            )
        };
        if success == 0 {
            return Err(std::io::Error::last_os_error())
                .context("setting the Windows internet options");
        }
        let changed =
            unsafe { InternetSetOptionW(null(), INTERNET_OPTION_SETTINGS_CHANGED, null(), 0) };
        let refreshed = unsafe { InternetSetOptionW(null(), INTERNET_OPTION_REFRESH, null(), 0) };
        if changed == 0 || refreshed == 0 {
            bail!(
                "Windows did not broadcast the system proxy change: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(())
    }

    pub(super) fn is_owned(current: &Snapshot, expected: &Snapshot) -> bool {
        current == expected
            || (current.flags & PROXY_TYPE_PROXY != 0
                && current.proxy_server == expected.proxy_server)
    }

    fn option(kind: u32) -> INTERNET_PER_CONN_OPTIONW {
        INTERNET_PER_CONN_OPTIONW {
            dwOption: kind,
            Value: INTERNET_PER_CONN_OPTIONW_0::default(),
        }
    }

    fn string_option(kind: u32, value: Option<&mut Vec<u16>>) -> INTERNET_PER_CONN_OPTIONW {
        INTERNET_PER_CONN_OPTIONW {
            dwOption: kind,
            Value: INTERNET_PER_CONN_OPTIONW_0 {
                pszValue: value.map_or(null_mut(), Vec::as_mut_ptr),
            },
        }
    }

    fn option_list(options: &mut [INTERNET_PER_CONN_OPTIONW]) -> INTERNET_PER_CONN_OPTION_LISTW {
        INTERNET_PER_CONN_OPTION_LISTW {
            dwSize: u32::try_from(size_of::<INTERNET_PER_CONN_OPTION_LISTW>()).unwrap(),
            pszConnection: null_mut(),
            dwOptionCount: u32::try_from(options.len()).unwrap(),
            dwOptionError: 0,
            pOptions: options.as_mut_ptr(),
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    unsafe fn take_wide(value: *mut u16) -> Option<String> {
        if value.is_null() {
            return None;
        }
        let mut length = 0;
        while unsafe { *value.add(length) } != 0 {
            length += 1;
        }
        let result = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(value, length) });
        unsafe {
            GlobalFree(value.cast());
        }
        Some(result)
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use std::net::SocketAddr;

    use anyhow::{Result, bail};
    use serde::{Deserialize, Serialize};

    #[derive(Deserialize, Serialize)]
    pub(super) struct Snapshot;

    pub(super) fn snapshot() -> Result<Snapshot> {
        bail!("system proxy mode is not supported on this platform")
    }

    pub(super) fn local_proxy(_listen: SocketAddr, _original: &Snapshot) -> Snapshot {
        Snapshot
    }

    pub(super) fn apply(_snapshot: &Snapshot) -> Result<()> {
        bail!("system proxy mode is not supported on this platform")
    }

    pub(super) fn is_owned(_current: &Snapshot, _expected: &Snapshot) -> bool {
        false
    }
}
