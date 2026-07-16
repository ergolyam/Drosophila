use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use yggdrasil::core::Core;
use yggdrasil::ipv6rwc::ReadWriteCloser;
use yggdrasil::tun::TunAdapter;

use crate::config::{StoredConfig, is_flatpak};
use crate::discovery::{DiscoveredPeer, discover_peers};
use crate::privileged::{PrivilegedNode, RemoteEnvelope, RemoteEvent, WorkerEvent};
use crate::proxy::UserspaceProxy;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeMode {
    Tun,
    Proxy,
}

#[derive(Debug)]
pub enum BackendEvent {
    Starting,
    Started {
        address: String,
        subnet: String,
        mode: NodeMode,
    },
    Stopped,
    Failed(String),
    PeerStatus(HashMap<String, bool>),
    DiscoveryFinished {
        id: u64,
        result: Result<Vec<DiscoveredPeer>, String>,
    },
}

enum BackendCommand {
    Start(Box<StoredConfig>),
    Stop,
    AddPeer(String),
    RemovePeer(String),
    Discover { id: u64, protocols: Vec<String> },
    Shutdown,
}

#[derive(Clone)]
pub struct BackendHandle {
    sender: mpsc::UnboundedSender<BackendCommand>,
}

impl BackendHandle {
    pub fn spawn() -> (Self, std_mpsc::Receiver<BackendEvent>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std_mpsc::channel();
        thread::Builder::new()
            .name("drosophila-network".to_owned())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .thread_name("drosophila-io")
                    .build()
                    .expect("failed to build Tokio runtime");
                runtime.block_on(backend_loop(command_rx, event_tx));
            })
            .expect("failed to start network thread");
        (Self { sender: command_tx }, event_rx)
    }

    pub fn start(&self, config: StoredConfig) {
        let _ = self.sender.send(BackendCommand::Start(Box::new(config)));
    }

    pub fn stop(&self) {
        let _ = self.sender.send(BackendCommand::Stop);
    }

    pub fn add_peer(&self, peer: String) {
        let _ = self.sender.send(BackendCommand::AddPeer(peer));
    }

    pub fn remove_peer(&self, peer: String) {
        let _ = self.sender.send(BackendCommand::RemovePeer(peer));
    }

    pub fn discover(&self, id: u64, protocols: Vec<String>) {
        let _ = self.sender.send(BackendCommand::Discover { id, protocols });
    }

    pub fn shutdown(&self) {
        let _ = self.sender.send(BackendCommand::Shutdown);
    }
}

async fn backend_loop(
    mut commands: mpsc::UnboundedReceiver<BackendCommand>,
    events: std_mpsc::Sender<BackendEvent>,
) {
    let mut node: Option<ActiveNode> = None;
    let (remote_tx, mut remote_rx) = mpsc::unbounded_channel::<RemoteEnvelope>();
    let mut next_remote_session = 0_u64;
    let mut status_timer = tokio::time::interval(Duration::from_secs(1));
    status_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                if !handle_command(
                    command,
                    &mut node,
                    &mut next_remote_session,
                    &remote_tx,
                    &events,
                ).await {
                    break;
                }
            }
            remote = remote_rx.recv() => {
                let Some(remote) = remote else { continue };
                handle_remote_event(remote, &mut node, &events);
            }
            _ = status_timer.tick(), if matches!(node, Some(ActiveNode::Local(_))) => {
                if let Some(ActiveNode::Local(current)) = &node {
                    let _ = events.send(BackendEvent::PeerStatus(current.peer_status().await));
                }
            }
        }
    }

    if let Some(current) = node {
        current.close().await;
    }
}

async fn handle_command(
    command: BackendCommand,
    node: &mut Option<ActiveNode>,
    next_remote_session: &mut u64,
    remote_tx: &mpsc::UnboundedSender<RemoteEnvelope>,
    events: &std_mpsc::Sender<BackendEvent>,
) -> bool {
    match command {
        BackendCommand::Start(config) => {
            if let Some(current) = node.take() {
                current.close().await;
            }
            let _ = events.send(BackendEvent::Starting);
            *node = start_node(*config, next_remote_session, remote_tx, events).await;
        }
        BackendCommand::Stop => {
            if let Some(current) = node.take() {
                current.close().await;
            }
            let _ = events.send(BackendEvent::Stopped);
        }
        BackendCommand::AddPeer(peer) => {
            if let Some(current) = node {
                match current {
                    ActiveNode::Local(current) => {
                        if let Err(error) = current.add_peer(&peer).await {
                            let _ = events
                                .send(BackendEvent::Failed(format!("Failed to add peer: {error}")));
                        }
                    }
                    ActiveNode::Privileged { node, .. } => node.add_peer(peer),
                }
            }
        }
        BackendCommand::RemovePeer(peer) => {
            if let Some(current) = node {
                match current {
                    ActiveNode::Local(current) => {
                        if let Err(error) = current.remove_peer(&peer).await {
                            let _ = events.send(BackendEvent::Failed(format!(
                                "Failed to remove peer: {error}"
                            )));
                        }
                    }
                    ActiveNode::Privileged { node, .. } => node.remove_peer(peer),
                }
            }
        }
        BackendCommand::Discover { id, protocols } => {
            let event_sender = events.clone();
            tokio::spawn(async move {
                let result = discover_peers(&protocols)
                    .await
                    .map_err(|error| format!("{error:#}"));
                let _ = event_sender.send(BackendEvent::DiscoveryFinished { id, result });
            });
        }
        BackendCommand::Shutdown => return false,
    }
    true
}

async fn start_node(
    config: StoredConfig,
    next_remote_session: &mut u64,
    remote_tx: &mpsc::UnboundedSender<RemoteEnvelope>,
    events: &std_mpsc::Sender<BackendEvent>,
) -> Option<ActiveNode> {
    if config.drosophila.proxy_enabled {
        match RunningNode::start(config).await {
            Ok(started) => {
                let _ = events.send(BackendEvent::Started {
                    address: started.address.clone(),
                    subnet: started.subnet.clone(),
                    mode: started.mode,
                });
                Some(ActiveNode::Local(started))
            }
            Err(error) => {
                tracing::error!(%error, "failed to start Yggdrasil-ng");
                let _ = events.send(BackendEvent::Failed(format!("{error:#}")));
                None
            }
        }
    } else {
        *next_remote_session = next_remote_session.wrapping_add(1);
        let session = *next_remote_session;
        match PrivilegedNode::launch(config, session, remote_tx.clone()).await {
            Ok(started) => Some(ActiveNode::Privileged {
                session,
                node: started,
            }),
            Err(error) => {
                tracing::error!(%error, "failed to authorize the TUN worker");
                let _ = events.send(BackendEvent::Failed(format!("{error:#}")));
                None
            }
        }
    }
}

fn handle_remote_event(
    remote: RemoteEnvelope,
    node: &mut Option<ActiveNode>,
    events: &std_mpsc::Sender<BackendEvent>,
) {
    let matching_session = matches!(
        node,
        Some(ActiveNode::Privileged { session, .. }) if *session == remote.session
    );
    if !matching_session {
        return;
    }
    match remote.event {
        RemoteEvent::Worker(WorkerEvent::Started { address, subnet }) => {
            let _ = events.send(BackendEvent::Started {
                address,
                subnet,
                mode: NodeMode::Tun,
            });
        }
        RemoteEvent::Worker(
            WorkerEvent::StartFailed { message } | WorkerEvent::OperationFailed { message },
        ) => {
            let _ = events.send(BackendEvent::Failed(message));
        }
        RemoteEvent::Worker(WorkerEvent::PeerStatus { statuses }) => {
            let _ = events.send(BackendEvent::PeerStatus(statuses));
        }
        RemoteEvent::Disconnected { error } => {
            *node = None;
            if let Some(error) = error {
                let _ = events.send(BackendEvent::Stopped);
                let _ = events.send(BackendEvent::Failed(error));
            }
        }
    }
}

enum ActiveNode {
    Local(RunningNode),
    Privileged { session: u64, node: PrivilegedNode },
}

impl ActiveNode {
    async fn close(self) {
        match self {
            Self::Local(node) => node.close().await,
            Self::Privileged { node, .. } => node.close().await,
        }
    }
}

pub(crate) struct RunningNode {
    core: Arc<Core>,
    tun: Option<TunAdapter>,
    proxy: Option<UserspaceProxy>,
    address: String,
    subnet: String,
    mode: NodeMode,
}

impl RunningNode {
    pub(crate) async fn start(mut stored: StoredConfig) -> Result<Self> {
        let mode = if stored.drosophila.proxy_enabled {
            NodeMode::Proxy
        } else {
            NodeMode::Tun
        };
        if mode == NodeMode::Tun && is_flatpak() {
            bail!(
                "Flatpak cannot create a host TUN interface. Enable the SOCKS Proxy mode in Settings."
            );
        }

        stored.yggdrasil.admin_listen = "none".to_owned();
        stored.yggdrasil.if_name = if mode == NodeMode::Tun {
            "auto".to_owned()
        } else {
            "none".to_owned()
        };
        let signing_key = stored
            .yggdrasil
            .signing_key()
            .map_err(|error| anyhow!(error))
            .context("invalid private key")?;

        let core = Core::new(signing_key, stored.yggdrasil.clone());
        core.init_links().await;
        core.start().await;

        let address = core.address().to_string();
        let subnet = core.subnet().to_string();
        let mtu = core.mtu();
        let rwc = ReadWriteCloser::new(core.clone(), mtu, None);
        core.set_path_notify(rwc.clone());

        if let Err(error) = core.start_multicast().await {
            tracing::warn!(%error, "multicast peer discovery is unavailable");
        }

        let result = match mode {
            NodeMode::Tun => {
                let tun_mtu = stored.yggdrasil.if_mtu.min(mtu).min(65_535) as u16;
                TunAdapter::new(
                    &stored.yggdrasil.if_name,
                    rwc,
                    &address,
                    &subnet,
                    tun_mtu,
                    #[cfg(windows)]
                    &stored.yggdrasil.if_dns_servers,
                )
                .await
                .map(|tun| (Some(tun), None))
                .map_err(anyhow::Error::msg)
            }
            NodeMode::Proxy => {
                let listen = stored
                    .drosophila
                    .proxy_listen
                    .parse::<SocketAddr>()
                    .context("invalid SOCKS listen address")?;
                let dns =
                    parse_dns_server(&stored.drosophila.dns_server, stored.drosophila.dns_port)?;
                let ipv6 = address
                    .parse::<Ipv6Addr>()
                    .context("Yggdrasil-ng returned an invalid IPv6 address")?;
                UserspaceProxy::start(rwc, ipv6, mtu as usize, listen, dns)
                    .await
                    .map(|proxy| (None, Some(proxy)))
            }
        };

        let (tun, proxy) = match result {
            Ok(value) => value,
            Err(error) => {
                core.close_multicast().await;
                let _ = core.close().await;
                return Err(error);
            }
        };

        Ok(Self {
            core,
            tun,
            proxy,
            address,
            subnet,
            mode,
        })
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    pub(crate) fn subnet(&self) -> &str {
        &self.subnet
    }

    pub(crate) async fn add_peer(&self, peer: &str) -> Result<(), String> {
        self.core.add_peer(peer).await
    }

    pub(crate) async fn remove_peer(&self, peer: &str) -> Result<(), String> {
        self.core.remove_peer(peer).await
    }

    pub(crate) async fn peer_status(&self) -> HashMap<String, bool> {
        self.core
            .get_peers()
            .await
            .into_iter()
            .map(|peer| (without_query(&peer.uri).to_owned(), peer.up))
            .collect()
    }

    pub(crate) async fn close(mut self) {
        if let Some(proxy) = self.proxy.take() {
            proxy.close().await;
        }
        self.core.close_multicast().await;
        let _ = self.core.close().await;
        if let Some(tun) = self.tun.take() {
            tun.close().await;
        }
    }
}

fn parse_dns_server(host: &str, port: u16) -> Result<Option<SocketAddr>> {
    let host = host.trim();
    if host.is_empty() {
        return Ok(None);
    }
    let ip = host
        .parse::<IpAddr>()
        .with_context(|| format!("invalid DNS server address {host}"))?;
    Ok(Some(match ip {
        IpAddr::V4(address) => SocketAddr::new(IpAddr::V4(address), port),
        IpAddr::V6(address) => SocketAddr::V6(SocketAddrV6::new(address, port, 0, 0)),
    }))
}

fn without_query(uri: &str) -> &str {
    uri.split_once('?').map_or(uri, |(base, _)| base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_parser_accepts_bare_ipv6() {
        let address = parse_dns_server("2001:db8::53", 53).unwrap().unwrap();
        assert_eq!(address, "[2001:db8::53]:53".parse().unwrap());
    }

    #[test]
    fn peer_status_key_ignores_query_parameters() {
        assert_eq!(
            without_query("tls://example:443?sni=x"),
            "tls://example:443"
        );
    }
}
