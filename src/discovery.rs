use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use futures::{StreamExt, stream};
use serde::Deserialize;
use tokio::net::TcpStream;
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
    #[serde(default)]
    response_ms: u64,
}

type PublicNodes = HashMap<String, HashMap<String, PeerMetadata>>;

pub async fn discover_peers(protocols: &[String]) -> Result<Vec<DiscoveredPeer>> {
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
    let mut available: Vec<_> = stream::iter(peers)
        .map(probe_peer)
        .buffer_unordered(20)
        .filter_map(|peer| async move { peer })
        .collect()
        .await;
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
                ping_ms: (metadata.response_ms > 0).then_some(metadata.response_ms),
            });
        }
    }
    peers
}

async fn probe_peer(mut peer: DiscoveredPeer) -> Option<DiscoveredPeer> {
    if peer.protocol == "quic" {
        return Some(peer);
    }

    let started = Instant::now();
    let connected = tokio::time::timeout(
        Duration::from_secs(3),
        TcpStream::connect((peer.host.as_str(), peer.port)),
    )
    .await
    .ok()?
    .ok()?;
    drop(connected);
    peer.ping_ms = Some(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX));
    Some(peer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_filter_protocol_and_apply_default_websocket_port() {
        let nodes: PublicNodes = serde_json::from_str(
            r#"{
                "DE.md": {
                    "wss://example.com/yggdrasil": {"up": true, "response_ms": 12},
                    "tcp://example.net:1234": {"up": true, "response_ms": 3}
                }
            }"#,
        )
        .unwrap();
        let peers = build_candidates(nodes, &["wss".to_owned()]);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].port, 443);
        assert_eq!(peers[0].country, "DE");
    }
}
