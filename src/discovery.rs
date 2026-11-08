use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::net::IpAddr;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use futures::{StreamExt, stream, stream::FuturesUnordered};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::net::lookup_host;
use tokio::process::Command;
use url::Url;

const SOURCES: [&str; 2] = [
    "https://publicpeers.neilalexander.dev/publicnodes.json",
    "https://peers.yggdrasil.link/publicnodes.json",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredPeer {
    pub address: String,
    pub protocol: String,
    pub host: String,
    pub port: u16,
    pub country: String,
    pub ping_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PeerMetadata {
    #[serde(default)]
    up: bool,
}

type PublicNodes = HashMap<String, HashMap<String, PeerMetadata>>;

pub async fn discover_peers(
    protocols: &[String],
    on_peer: impl Fn(DiscoveredPeer) + Send + Sync,
) -> Result<Vec<DiscoveredPeer>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("Drosophila/", env!("CARGO_PKG_VERSION")))
        .build()
        .context("creating peer discovery HTTP client")?;

    let mut last_error = None;
    let mut nodes = None;
    for source in SOURCES {
        match client.get(source).send().await {
            Ok(response) => match response.error_for_status() {
                Ok(response) => match response.json::<PublicNodes>().await {
                    Ok(value) => {
                        nodes = Some(value);
                        break;
                    }
                    Err(error) => last_error = Some(anyhow!(error)),
                },
                Err(error) => last_error = Some(anyhow!(error)),
            },
            Err(error) => last_error = Some(anyhow!(error)),
        }
    }
    let nodes = nodes
        .ok_or_else(|| last_error.unwrap_or_else(|| anyhow!("all public peer sources failed")))?;

    let peers = build_candidates(nodes, protocols);
    let executable = std::env::current_exe().context("locating peer probe executable")?;
    check_candidates(
        peers,
        |peer| probe_peer(peer, &executable),
        |peer| async move { ping_peer(&peer).await },
        on_peer,
    )
    .await
}

async fn check_candidates<P, PF, I, IF, C>(
    peers: Vec<DiscoveredPeer>,
    probe: P,
    ping: I,
    on_peer: C,
) -> Result<Vec<DiscoveredPeer>>
where
    P: Fn(DiscoveredPeer) -> PF,
    PF: Future<Output = Result<Option<DiscoveredPeer>>>,
    I: Fn(DiscoveredPeer) -> IF,
    IF: Future<Output = Option<u64>>,
    C: Fn(DiscoveredPeer),
{
    let probes = stream::iter(peers).map(probe).buffer_unordered(32);
    futures::pin_mut!(probes);
    let mut pending_pings = VecDeque::new();
    let mut pings = FuturesUnordered::new();
    let mut probes_finished = false;
    let mut available = Vec::new();
    loop {
        while pings.len() < 32 {
            let Some(mut peer): Option<DiscoveredPeer> = pending_pings.pop_front() else {
                break;
            };
            let future = ping(peer.clone());
            pings.push(async move {
                peer.ping_ms = future.await;
                peer
            });
        }
        if probes_finished && pings.is_empty() && pending_pings.is_empty() {
            break;
        }
        tokio::select! {
            result = probes.next(), if !probes_finished => match result {
                Some(Ok(Some(peer))) => {
                    on_peer(peer.clone());
                    pending_pings.push_back(peer);
                }
                Some(Ok(None)) => {}
                Some(Err(error)) => return Err(error),
                None => probes_finished = true,
            },
            Some(peer) = pings.next(), if !pings.is_empty() => {
                if peer.ping_ms.is_some() {
                    on_peer(peer.clone());
                }
                available.push(peer);
            }
        }
    }
    available.sort_by_key(|peer| (peer.ping_ms.unwrap_or(u64::MAX), peer.country.clone()));
    Ok(available)
}

fn build_candidates(nodes: PublicNodes, protocols: &[String]) -> Vec<DiscoveredPeer> {
    let wanted: HashSet<&str> = protocols.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut peers = Vec::new();
    for (region, entries) in nodes {
        let country = region.trim_end_matches(".md").to_owned();
        for (address, metadata) in entries {
            if !metadata.up {
                continue;
            }
            if !seen.insert(address.clone()) {
                continue;
            }
            if address.chars().any(char::is_control) || address.len() > 8192 {
                continue;
            }
            let Ok(parsed) = Url::parse(&address) else {
                continue;
            };
            if !wanted.contains(parsed.scheme()) {
                continue;
            }
            let Some(host) = parsed.host_str() else {
                continue;
            };
            let Some(port) = parsed.port_or_known_default() else {
                continue;
            };
            peers.push(DiscoveredPeer {
                address,
                protocol: parsed.scheme().to_owned(),
                host: host.to_owned(),
                port,
                country: country.clone(),
                ping_ms: None,
            });
        }
    }
    peers
}

async fn probe_peer(
    peer: DiscoveredPeer,
    executable: &std::path::Path,
) -> Result<Option<DiscoveredPeer>> {
    if !probe_yggdrasil(executable, &peer.address).await? {
        return Ok(None);
    }
    Ok(Some(peer))
}

async fn probe_yggdrasil(executable: &std::path::Path, address: &str) -> Result<bool> {
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        let mut command = Command::new(executable);
        command
            .arg("--peer-probe")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let mut child = command.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(address.as_bytes()).await.is_err() || stdin.shutdown().await.is_err()
            {
                return Ok(false);
            }
        }
        child.wait().await.map(|status| status.success())
    })
    .await;
    match result {
        Ok(result) => result.context("running peer probe"),
        Err(_) => Ok(false),
    }
}

async fn ping_peer(peer: &DiscoveredPeer) -> Option<u64> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let host = peer.host.trim_start_matches('[').trim_end_matches(']');
        let addresses = lookup_host((host, peer.port)).await.ok()?;
        for address in addresses {
            if let Some(ping) = ping_address(address.ip()).await {
                return Some(ping);
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}

async fn ping_address(address: IpAddr) -> Option<u64> {
    #[cfg(target_os = "linux")]
    let mut command = if std::env::var_os("FLATPAK_ID").is_some() {
        let mut command = Command::new("flatpak-spawn");
        command.args(["--host", "--watch-bus", "--env=LC_ALL=C", "ping"]);
        command
    } else {
        Command::new("ping")
    };
    #[cfg(not(target_os = "linux"))]
    let mut command = Command::new("ping");
    command.kill_on_drop(true);
    command.env("LC_ALL", "C");
    #[cfg(windows)]
    {
        command.creation_flags(0x0800_0000);
        command.args(["-n", "1", "-w", "3000"]);
    }
    #[cfg(not(windows))]
    command.args(["-n", "-c", "1", "-W", "3"]);
    command.arg(address.to_string());
    let output = command.output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    parse_ping_output(&String::from_utf8_lossy(&output.stdout))
}

fn parse_ping_output(output: &str) -> Option<u64> {
    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(position) = lower.find("time=").or_else(|| lower.find("time<")) {
            let value = lower[position + 5..].trim_start();
            if let Some(end) =
                value.find(|character: char| !character.is_ascii_digit() && character != '.')
                && value[end..].trim_start().starts_with("ms")
                && let Some(millis) = parse_millis(&value[..end])
            {
                return Some(millis);
            }
        }
        #[cfg(windows)]
        if let Some(position) = lower.find("ttl=") {
            let prefix = &lower[..position];
            if let Some(separator) = prefix.rfind(['=', '<']) {
                let value = prefix[separator + 1..].trim_start();
                let end = value
                    .find(|character: char| {
                        !character.is_ascii_digit() && character != '.' && character != ','
                    })
                    .unwrap_or(value.len());
                if let Some(millis) = parse_millis(&value[..end].replace(',', ".")) {
                    return Some(millis);
                }
            }
        }
    }
    None
}

fn parse_millis(number: &str) -> Option<u64> {
    let millis: f64 = number.parse().ok()?;
    (millis.is_finite() && millis >= 0.0 && millis < u64::MAX as f64).then(|| millis.ceil() as u64)
}

#[cfg(test)]
#[path = "tests/discovery.rs"]
mod tests;
