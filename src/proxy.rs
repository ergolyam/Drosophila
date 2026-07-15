use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use futures::{Sink, Stream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_smoltcp::device::{AsyncDevice, DeviceCapabilities, Packet};
use tokio_smoltcp::smoltcp::iface::Config as InterfaceConfig;
use tokio_smoltcp::smoltcp::phy::Medium;
use tokio_smoltcp::smoltcp::wire::{HardwareAddress, IpAddress, IpCidr, Ipv6Address};
use tokio_smoltcp::{Net, NetConfig};
use tokio_util::sync::{CancellationToken, PollSender};
use yggdrasil::ipv6rwc::ReadWriteCloser;

const SOCKS_VERSION: u8 = 5;
const SOCKS_CONNECT: u8 = 1;
const ADDRESS_IPV4: u8 = 1;
const ADDRESS_DOMAIN: u8 = 3;
const ADDRESS_IPV6: u8 = 4;

pub struct UserspaceProxy {
    cancel: CancellationToken,
    server: JoinHandle<()>,
    bridge_handles: Vec<JoinHandle<()>>,
    _net: Arc<Net>,
}

impl UserspaceProxy {
    pub async fn start(
        rwc: Arc<ReadWriteCloser>,
        address: Ipv6Addr,
        mtu: usize,
        listen: SocketAddr,
        dns_server: Option<SocketAddr>,
    ) -> Result<Self> {
        let (device, bridge_handles) = RwcDevice::new(rwc, mtu);

        let ip = IpAddress::Ipv6(Ipv6Address::from(address.octets()));
        let mut interface = InterfaceConfig::new(HardwareAddress::Ip);
        interface.random_seed = rand::random();
        let config = NetConfig::new(interface, IpCidr::new(ip, 7), vec![ip]);
        let net = Arc::new(Net::new(device, config));

        let listener = TcpListener::bind(listen)
            .await
            .with_context(|| format!("binding SOCKS5 proxy to {listen}"))?;
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task_net = net.clone();
        let server = tokio::spawn(async move {
            let mut clients = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    () = task_cancel.cancelled() => break,
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                let net = task_net.clone();
                                clients.spawn(async move {
                                    if let Err(error) = serve_client(stream, net, dns_server).await {
                                        tracing::debug!(%error, "SOCKS client disconnected");
                                    }
                                });
                            }
                            Err(error) => {
                                tracing::warn!(%error, "SOCKS accept failed");
                                break;
                            }
                        }
                    }
                    Some(result) = clients.join_next(), if !clients.is_empty() => {
                        if let Err(error) = result {
                            tracing::debug!(%error, "SOCKS client task failed");
                        }
                    }
                }
            }
            clients.abort_all();
            while clients.join_next().await.is_some() {}
        });

        Ok(Self {
            cancel,
            server,
            bridge_handles,
            _net: net,
        })
    }

    pub async fn close(mut self) {
        self.cancel.cancel();
        if tokio::time::timeout(Duration::from_secs(2), &mut self.server)
            .await
            .is_err()
        {
            self.server.abort();
            let _ = self.server.await;
        }
        for handle in &self.bridge_handles {
            handle.abort();
        }
        for handle in self.bridge_handles {
            let _ = handle.await;
        }
    }
}

struct RwcDevice {
    incoming: mpsc::Receiver<io::Result<Packet>>,
    outgoing: PollSender<Packet>,
    capabilities: DeviceCapabilities,
}

impl RwcDevice {
    fn new(rwc: Arc<ReadWriteCloser>, mtu: usize) -> (Self, Vec<JoinHandle<()>>) {
        let (incoming_tx, incoming) = mpsc::channel(128);
        let (outgoing_tx, mut outgoing_rx) = mpsc::channel::<Packet>(128);

        let reader_rwc = rwc.clone();
        let reader = tokio::spawn(async move {
            let mut buffer = vec![0_u8; 65_535];
            loop {
                match reader_rwc.read(&mut buffer).await {
                    Ok(size) => {
                        if incoming_tx.send(Ok(buffer[..size].to_vec())).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = incoming_tx.send(Err(io::Error::other(error))).await;
                        break;
                    }
                }
            }
        });

        let writer = tokio::spawn(async move {
            while let Some(packet) = outgoing_rx.recv().await {
                if let Err(error) = rwc.write(&packet).await {
                    tracing::debug!(%error, "userspace stack stopped writing packets");
                    break;
                }
            }
        });

        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ip;
        capabilities.max_transmission_unit = mtu;
        capabilities.max_burst_size = Some(32);

        (
            Self {
                incoming,
                outgoing: PollSender::new(outgoing_tx),
                capabilities,
            },
            vec![reader, writer],
        )
    }
}

impl Stream for RwcDevice {
    type Item = io::Result<Packet>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.incoming.poll_recv(cx)
    }
}

impl Sink<Packet> for RwcDevice {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.outgoing
            .poll_reserve(cx)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "packet channel closed"))
    }

    fn start_send(mut self: Pin<&mut Self>, packet: Packet) -> Result<(), Self::Error> {
        self.outgoing
            .send_item(packet)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "packet channel closed"))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.outgoing.close();
        Poll::Ready(Ok(()))
    }
}

impl AsyncDevice for RwcDevice {
    fn capabilities(&self) -> &DeviceCapabilities {
        &self.capabilities
    }
}

async fn serve_client(
    mut client: TcpStream,
    net: Arc<Net>,
    dns_server: Option<SocketAddr>,
) -> Result<()> {
    negotiate_authentication(&mut client).await?;
    let target = read_request(&mut client, &net, dns_server).await?;

    let mut remote = match net.tcp_connect(target).await {
        Ok(stream) => stream,
        Err(error) => {
            send_reply(&mut client, 5).await?;
            return Err(error).context("connecting through Yggdrasil");
        }
    };
    send_reply(&mut client, 0).await?;
    tokio::io::copy_bidirectional(&mut client, &mut remote)
        .await
        .context("proxying SOCKS5 stream")?;
    Ok(())
}

async fn negotiate_authentication(client: &mut TcpStream) -> Result<()> {
    let mut header = [0_u8; 2];
    client.read_exact(&mut header).await?;
    if header[0] != SOCKS_VERSION || header[1] == 0 {
        bail!("invalid SOCKS5 greeting");
    }
    let mut methods = vec![0_u8; usize::from(header[1])];
    client.read_exact(&mut methods).await?;
    let selected = if methods.contains(&0) { 0 } else { 0xff };
    client.write_all(&[SOCKS_VERSION, selected]).await?;
    if selected == 0xff {
        bail!("SOCKS5 client does not support no-auth mode");
    }
    Ok(())
}

async fn read_request(
    client: &mut TcpStream,
    net: &Arc<Net>,
    dns_server: Option<SocketAddr>,
) -> Result<SocketAddr> {
    let mut header = [0_u8; 4];
    client.read_exact(&mut header).await?;
    if header[0] != SOCKS_VERSION || header[1] != SOCKS_CONNECT || header[2] != 0 {
        send_reply(client, 7).await?;
        bail!("only SOCKS5 CONNECT is supported");
    }

    let host = match header[3] {
        ADDRESS_IPV4 => {
            let mut ignored = [0_u8; 4];
            client.read_exact(&mut ignored).await?;
            send_reply(client, 8).await?;
            bail!("Yggdrasil only routes IPv6 destinations");
        }
        ADDRESS_IPV6 => {
            let mut bytes = [0_u8; 16];
            client.read_exact(&mut bytes).await?;
            TargetHost::Address(Ipv6Addr::from(bytes))
        }
        ADDRESS_DOMAIN => {
            let length = client.read_u8().await?;
            let mut bytes = vec![0_u8; usize::from(length)];
            client.read_exact(&mut bytes).await?;
            TargetHost::Domain(String::from_utf8(bytes).context("invalid domain name")?)
        }
        _ => {
            send_reply(client, 8).await?;
            bail!("unsupported SOCKS5 address type");
        }
    };
    let port = client.read_u16().await?;

    let address = match host {
        TargetHost::Address(address) => address,
        TargetHost::Domain(domain) => resolve_ipv6(net, &domain, dns_server).await?,
    };
    Ok(SocketAddr::V6(SocketAddrV6::new(address, port, 0, 0)))
}

enum TargetHost {
    Address(Ipv6Addr),
    Domain(String),
}

async fn send_reply(client: &mut TcpStream, status: u8) -> io::Result<()> {
    let mut reply = [0_u8; 22];
    reply[0] = SOCKS_VERSION;
    reply[1] = status;
    reply[3] = ADDRESS_IPV6;
    client.write_all(&reply).await
}

async fn resolve_ipv6(
    net: &Arc<Net>,
    domain: &str,
    dns_server: Option<SocketAddr>,
) -> Result<Ipv6Addr> {
    if let Ok(address) = domain.parse::<Ipv6Addr>() {
        return Ok(address);
    }

    if let Some(server) = dns_server {
        return resolve_with_overlay_dns(net, domain, server).await;
    }

    let addresses = tokio::net::lookup_host((domain, 0))
        .await
        .with_context(|| format!("resolving {domain}"))?;
    addresses
        .filter_map(|address| match address.ip() {
            IpAddr::V6(address) => Some(address),
            IpAddr::V4(_) => None,
        })
        .next()
        .ok_or_else(|| anyhow!("{domain} has no IPv6 address"))
}

async fn resolve_with_overlay_dns(
    net: &Arc<Net>,
    domain: &str,
    server: SocketAddr,
) -> Result<Ipv6Addr> {
    if !server.is_ipv6() {
        bail!("the overlay DNS server must be an IPv6 address");
    }
    let query_id = rand::random::<u16>();
    let query = build_aaaa_query(query_id, domain)?;
    let socket = net.udp_bind("[::]:0".parse().unwrap()).await?;
    socket.send_to(&query, server).await?;

    let mut response = vec![0_u8; 4096];
    let (size, _) = tokio::time::timeout(Duration::from_secs(5), socket.recv_from(&mut response))
        .await
        .context("overlay DNS query timed out")??;
    parse_aaaa_response(query_id, &response[..size])
        .ok_or_else(|| anyhow!("overlay DNS returned no AAAA record for {domain}"))
}

fn build_aaaa_query(id: u16, domain: &str) -> Result<Vec<u8>> {
    let mut packet = Vec::with_capacity(512);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&0x0100_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes());
    for label in domain.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            bail!("invalid DNS name {domain}");
        }
        packet.push(u8::try_from(label.len()).context("DNS label is too long")?);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.push(0);
    packet.extend_from_slice(&28_u16.to_be_bytes());
    packet.extend_from_slice(&1_u16.to_be_bytes());
    Ok(packet)
}

fn parse_aaaa_response(id: u16, packet: &[u8]) -> Option<Ipv6Addr> {
    if packet.len() < 12 || u16::from_be_bytes([packet[0], packet[1]]) != id {
        return None;
    }
    let questions = usize::from(u16::from_be_bytes([packet[4], packet[5]]));
    let answers = usize::from(u16::from_be_bytes([packet[6], packet[7]]));
    let mut offset = 12;
    for _ in 0..questions {
        skip_dns_name(packet, &mut offset)?;
        offset = offset.checked_add(4)?;
        if offset > packet.len() {
            return None;
        }
    }
    for _ in 0..answers {
        skip_dns_name(packet, &mut offset)?;
        if offset.checked_add(10)? > packet.len() {
            return None;
        }
        let record_type = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let class = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
        let length = usize::from(u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]));
        offset += 10;
        if offset.checked_add(length)? > packet.len() {
            return None;
        }
        if record_type == 28 && class == 1 && length == 16 {
            let bytes: [u8; 16] = packet[offset..offset + 16].try_into().ok()?;
            return Some(Ipv6Addr::from(bytes));
        }
        offset += length;
    }
    None
}

fn skip_dns_name(packet: &[u8], offset: &mut usize) -> Option<()> {
    loop {
        let length = *packet.get(*offset)?;
        *offset += 1;
        if length == 0 {
            return Some(());
        }
        if length & 0xc0 == 0xc0 {
            *offset = offset.checked_add(1)?;
            return (*offset <= packet.len()).then_some(());
        }
        if length & 0xc0 != 0 {
            return None;
        }
        *offset = offset.checked_add(usize::from(length))?;
        if *offset > packet.len() {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns_query_encodes_aaaa_question() {
        let query = build_aaaa_query(0x1234, "example.test").unwrap();
        assert_eq!(&query[..2], &[0x12, 0x34]);
        assert!(query.windows(7).any(|window| window == b"example"));
        assert_eq!(&query[query.len() - 4..], &[0, 28, 0, 1]);
    }
}
