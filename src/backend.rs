use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;
use yggdrasil::core::Core;
use yggdrasil::ipv6rwc::ReadWriteCloser;
#[cfg(feature = "tun")]
use yggdrasil::tun::TunAdapter;

use crate::config::{ConnectionMode, StoredConfig, is_flatpak};
use crate::discovery::{DiscoveredPeer, discover_peers};
use crate::privileged::{PrivilegedNode, RemoteEnvelope, RemoteEvent, WorkerEvent};
use crate::proxy::UserspaceProxy;
use crate::system_proxy::SystemProxy;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeMode {
    Tun,
    SystemProxy,
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
    inner: Arc<BackendHandleInner>,
}

struct BackendHandleInner {
    sender: mpsc::UnboundedSender<BackendCommand>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl BackendHandle {
    pub fn spawn() -> (Self, std_mpsc::Receiver<BackendEvent>) {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std_mpsc::channel();
        let thread = thread::Builder::new()
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
        (
            Self {
                inner: Arc::new(BackendHandleInner {
                    sender: command_tx,
                    thread: Mutex::new(Some(thread)),
                }),
            },
            event_rx,
        )
    }

    pub fn start(&self, config: StoredConfig) {
        let _ = self
            .inner
            .sender
            .send(BackendCommand::Start(Box::new(config)));
    }

    pub fn stop(&self) {
        let _ = self.inner.sender.send(BackendCommand::Stop);
    }

    pub fn add_peer(&self, peer: String) {
        let _ = self.inner.sender.send(BackendCommand::AddPeer(peer));
    }

    pub fn remove_peer(&self, peer: String) {
        let _ = self.inner.sender.send(BackendCommand::RemovePeer(peer));
    }

    pub fn discover(&self, id: u64, protocols: Vec<String>) {
        let _ = self
            .inner
            .sender
            .send(BackendCommand::Discover { id, protocols });
    }

    pub fn shutdown(&self) {
        let _ = self.inner.sender.send(BackendCommand::Shutdown);
        let thread = self
            .inner
            .thread
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
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
    if config.drosophila.effective_mode() == ConnectionMode::Tun {
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
    } else {
        match RunningNode::start(config).await {
            Ok(started) => {
                let _ = events.send(BackendEvent::Started {
                    address: started.address.clone(),
                    subnet: started.subnet.clone(),
                    mode: started.mode,
                });
                Some(ActiveNode::Local(Box::new(started)))
            }
            Err(error) => {
                tracing::error!(%error, "failed to start Yggdrasil-ng");
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
    Local(Box<RunningNode>),
    Privileged { session: u64, node: PrivilegedNode },
}

impl ActiveNode {
    async fn close(self) {
        match self {
            Self::Local(node) => (*node).close().await,
            Self::Privileged { node, .. } => node.close().await,
        }
    }
}

pub(crate) struct RunningNode {
    core: Arc<Core>,
    #[cfg(feature = "tun")]
    tun: Option<TunAdapter>,
    proxy: Option<UserspaceProxy>,
    system_proxy: Option<SystemProxy>,
    address: String,
    subnet: String,
    mode: NodeMode,
}

struct StartedTransports {
    #[cfg(feature = "tun")]
    tun: Option<TunAdapter>,
    proxy: Option<UserspaceProxy>,
    system_proxy: Option<SystemProxy>,
}

impl RunningNode {
    pub(crate) async fn start(mut stored: StoredConfig) -> Result<Self> {
        let mode = match stored.drosophila.effective_mode() {
            ConnectionMode::SystemProxy => NodeMode::SystemProxy,
            ConnectionMode::Proxy => NodeMode::Proxy,
            ConnectionMode::Tun => NodeMode::Tun,
        };
        if mode == NodeMode::Tun && is_flatpak() {
            bail!("Flatpak cannot create a host TUN interface. Select System Proxy in Settings.");
        }

        "none".clone_into(&mut stored.yggdrasil.admin_listen);
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
            #[cfg(feature = "tun")]
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
                .map(|tun| StartedTransports {
                    tun: Some(tun),
                    proxy: None,
                    system_proxy: None,
                })
                .map_err(anyhow::Error::msg)
            }
            #[cfg(not(feature = "tun"))]
            NodeMode::Tun => Err(anyhow!("this build does not include TUN support")),
            NodeMode::SystemProxy => start_proxy(rwc, &address, mtu, &stored, true).await,
            NodeMode::Proxy => start_proxy(rwc, &address, mtu, &stored, false).await,
        };

        let transports = match result {
            Ok(value) => value,
            Err(error) => {
                core.close_multicast().await;
                let _ = core.close().await;
                return Err(error);
            }
        };

        Ok(Self {
            core,
            #[cfg(feature = "tun")]
            tun: transports.tun,
            proxy: transports.proxy,
            system_proxy: transports.system_proxy,
            address,
            subnet,
            mode,
        })
    }

    #[cfg(feature = "tun")]
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    #[cfg(feature = "tun")]
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
        if let Some(system_proxy) = self.system_proxy.take() {
            system_proxy.close();
        }
        if let Some(proxy) = self.proxy.take() {
            proxy.close().await;
        }
        self.core.close_multicast().await;
        let _ = self.core.close().await;
        #[cfg(feature = "tun")]
        if let Some(tun) = self.tun.take() {
            tun.close().await;
        }
    }
}

async fn start_proxy(
    rwc: Arc<ReadWriteCloser>,
    address: &str,
    mtu: u64,
    stored: &StoredConfig,
    configure_system: bool,
) -> Result<StartedTransports> {
    let listen = stored
        .drosophila
        .proxy_listen
        .parse::<SocketAddr>()
        .context("invalid proxy listen address")?;
    let dns = parse_dns_server(&stored.drosophila.dns_server, stored.drosophila.dns_port)?;
    let ipv6 = address
        .parse::<Ipv6Addr>()
        .context("Yggdrasil-ng returned an invalid IPv6 address")?;
    let proxy = UserspaceProxy::start(
        rwc,
        ipv6,
        usize::try_from(mtu).context("Yggdrasil MTU does not fit this platform")?,
        listen,
        dns,
    )
    .await?;
    if !configure_system {
        return Ok(StartedTransports {
            #[cfg(feature = "tun")]
            tun: None,
            proxy: Some(proxy),
            system_proxy: None,
        });
    }
    match SystemProxy::enable(listen) {
        Ok(system_proxy) => Ok(StartedTransports {
            #[cfg(feature = "tun")]
            tun: None,
            proxy: Some(proxy),
            system_proxy: Some(system_proxy),
        }),
        Err(error) => {
            proxy.close().await;
            Err(error)
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
#[path = "tests/backend.rs"]
mod tests;
