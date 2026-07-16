use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};
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
use url::{Host, Url};
use yggdrasil::ipv6rwc::ReadWriteCloser;

const SOCKS_VERSION: u8 = 5;
const SOCKS_CONNECT: u8 = 1;
const ADDRESS_IPV4: u8 = 1;
const ADDRESS_DOMAIN: u8 = 3;
const ADDRESS_IPV6: u8 = 4;
const HTTP_HEADER_LIMIT: usize = 64 * 1024;

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
            .with_context(|| format!("binding local proxy to {listen}"))?;
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
                                        tracing::debug!(%error, "proxy client disconnected");
                                    }
                                });
                            }
                            Err(error) => {
                                tracing::warn!(%error, "proxy accept failed");
                                break;
                            }
                        }
                    }
                    Some(result) = clients.join_next(), if !clients.is_empty() => {
                        if let Err(error) = result {
                            tracing::debug!(%error, "proxy client task failed");
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
    client: TcpStream,
    net: Arc<Net>,
    dns_server: Option<SocketAddr>,
) -> Result<()> {
    let mut protocol = [0_u8; 1];
    if client.peek(&mut protocol).await? == 0 {
        bail!("proxy client closed before sending a request");
    }
    if protocol[0] == SOCKS_VERSION {
        serve_socks(client, net, dns_server).await
    } else {
        serve_http(client, net, dns_server).await
    }
}

async fn serve_socks(
    mut client: TcpStream,
    net: Arc<Net>,
    dns_server: Option<SocketAddr>,
) -> Result<()> {
    negotiate_authentication(&mut client).await?;
    let target = read_socks_request(&mut client).await?;
    let overlay = resolve_overlay_target(&net, &target, dns_server).await;

    if let Some(target) = overlay {
        let mut remote = match net.tcp_connect(target).await {
            Ok(stream) => stream,
            Err(error) => {
                send_socks_reply(&mut client, 5).await?;
                return Err(error).context("connecting through Yggdrasil");
            }
        };
        send_socks_reply(&mut client, 0).await?;
        tokio::io::copy_bidirectional(&mut client, &mut remote)
            .await
            .context("proxying a SOCKS5 overlay stream")?;
    } else {
        let mut remote = match connect_direct(&target).await {
            Ok(stream) => stream,
            Err(error) => {
                send_socks_reply(&mut client, 5).await?;
                return Err(error).context("connecting directly");
            }
        };
        send_socks_reply(&mut client, 0).await?;
        tokio::io::copy_bidirectional(&mut client, &mut remote)
            .await
            .context("proxying a direct SOCKS5 stream")?;
    }
    Ok(())
}

async fn serve_http(
    mut client: TcpStream,
    net: Arc<Net>,
    dns_server: Option<SocketAddr>,
) -> Result<()> {
    let request = read_http_request(&mut client).await?;
    let overlay = resolve_overlay_target(&net, &request.target, dns_server).await;

    if let Some(target) = overlay {
        let mut remote = match net.tcp_connect(target).await {
            Ok(stream) => stream,
            Err(error) => {
                send_http_error(&mut client).await?;
                return Err(error).context("connecting HTTP through Yggdrasil");
            }
        };
        start_http_proxying(&mut client, &mut remote, &request).await?;
        tokio::io::copy_bidirectional(&mut client, &mut remote)
            .await
            .context("proxying an HTTP overlay stream")?;
    } else {
        let mut remote = match connect_direct(&request.target).await {
            Ok(stream) => stream,
            Err(error) => {
                send_http_error(&mut client).await?;
                return Err(error).context("connecting HTTP directly");
            }
        };
        start_http_proxying(&mut client, &mut remote, &request).await?;
        tokio::io::copy_bidirectional(&mut client, &mut remote)
            .await
            .context("proxying a direct HTTP stream")?;
    }
    Ok(())
}

async fn start_http_proxying<R>(
    client: &mut TcpStream,
    remote: &mut R,
    request: &HttpRequest,
) -> Result<()>
where
    R: tokio::io::AsyncWrite + Unpin,
{
    match request.kind {
        HttpRequestKind::Tunnel => {
            client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .context("acknowledging an HTTP CONNECT request")?;
            remote
                .write_all(&request.buffered_body)
                .await
                .context("forwarding buffered tunnel data")?;
        }
        HttpRequestKind::Forward => {
            remote
                .write_all(&request.forwarded_head)
                .await
                .context("forwarding HTTP request headers")?;
            remote
                .write_all(&request.buffered_body)
                .await
                .context("forwarding buffered HTTP request data")?;
        }
    }
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

async fn read_socks_request(client: &mut TcpStream) -> Result<ProxyTarget> {
    let mut header = [0_u8; 4];
    client.read_exact(&mut header).await?;
    if header[0] != SOCKS_VERSION || header[1] != SOCKS_CONNECT || header[2] != 0 {
        send_socks_reply(client, 7).await?;
        bail!("only SOCKS5 CONNECT is supported");
    }

    let host = match header[3] {
        ADDRESS_IPV4 => {
            let mut bytes = [0_u8; 4];
            client.read_exact(&mut bytes).await?;
            TargetHost::Address(IpAddr::V4(Ipv4Addr::from(bytes)))
        }
        ADDRESS_IPV6 => {
            let mut bytes = [0_u8; 16];
            client.read_exact(&mut bytes).await?;
            TargetHost::Address(IpAddr::V6(Ipv6Addr::from(bytes)))
        }
        ADDRESS_DOMAIN => {
            let length = client.read_u8().await?;
            let mut bytes = vec![0_u8; usize::from(length)];
            client.read_exact(&mut bytes).await?;
            TargetHost::Domain(String::from_utf8(bytes).context("invalid domain name")?)
        }
        _ => {
            send_socks_reply(client, 8).await?;
            bail!("unsupported SOCKS5 address type");
        }
    };
    let port = client.read_u16().await?;
    Ok(ProxyTarget { host, port })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProxyTarget {
    host: TargetHost,
    port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TargetHost {
    Address(IpAddr),
    Domain(String),
}

async fn send_socks_reply(client: &mut TcpStream, status: u8) -> io::Result<()> {
    let mut reply = [0_u8; 22];
    reply[0] = SOCKS_VERSION;
    reply[1] = status;
    reply[3] = ADDRESS_IPV6;
    client.write_all(&reply).await
}

async fn read_http_request(client: &mut TcpStream) -> Result<HttpRequest> {
    let mut bytes = Vec::with_capacity(4096);
    let header_end = loop {
        if let Some(end) = find_header_end(&bytes) {
            break end;
        }
        if bytes.len() >= HTTP_HEADER_LIMIT {
            bail!("HTTP proxy request headers are too large");
        }
        let mut buffer = [0_u8; 4096];
        let size = client.read(&mut buffer).await?;
        if size == 0 {
            bail!("HTTP proxy client closed during request headers");
        }
        bytes.extend_from_slice(&buffer[..size]);
    };
    parse_http_request(bytes, header_end)
}

#[derive(Debug)]
struct HttpRequest {
    target: ProxyTarget,
    kind: HttpRequestKind,
    forwarded_head: Vec<u8>,
    buffered_body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HttpRequestKind {
    Tunnel,
    Forward,
}

fn parse_http_request(mut bytes: Vec<u8>, header_end: usize) -> Result<HttpRequest> {
    let buffered_body = bytes.split_off(header_end);
    let text = std::str::from_utf8(&bytes).context("HTTP proxy headers are not UTF-8")?;
    let mut lines = text[..text.len() - 4].split("\r\n");
    let request_line = lines.next().context("HTTP proxy request is empty")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().context("HTTP proxy request has no method")?;
    let request_target = parts.next().context("HTTP proxy request has no target")?;
    let version = parts.next().context("HTTP proxy request has no version")?;
    ensure_http_version(version)?;
    ensure_no_extra_request_line_parts(parts)?;

    if method.eq_ignore_ascii_case("CONNECT") {
        return Ok(HttpRequest {
            target: parse_authority(request_target)?,
            kind: HttpRequestKind::Tunnel,
            forwarded_head: Vec::new(),
            buffered_body,
        });
    }

    let url = Url::parse(request_target).context("HTTP proxy target is not an absolute URL")?;
    if url.scheme() != "http" {
        bail!("only HTTP URLs and CONNECT tunnels are supported");
    }
    let port = url
        .port_or_known_default()
        .context("HTTP proxy target has no port")?;
    let target = match url.host().context("HTTP proxy target has no host")? {
        Host::Ipv4(address) => ProxyTarget {
            host: TargetHost::Address(IpAddr::V4(address)),
            port,
        },
        Host::Ipv6(address) => ProxyTarget {
            host: TargetHost::Address(IpAddr::V6(address)),
            port,
        },
        Host::Domain(domain) => ProxyTarget {
            host: TargetHost::Domain(domain.to_owned()),
            port,
        },
    };
    let path = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let origin = url
        .query()
        .map_or_else(|| path.to_owned(), |query| format!("{path}?{query}"));
    let mut forwarded_head = format!("{method} {origin} {version}\r\n").into_bytes();
    for line in lines {
        let name = line.split_once(':').map_or(line, |(name, _)| name);
        if name.eq_ignore_ascii_case("proxy-connection") {
            continue;
        }
        forwarded_head.extend_from_slice(line.as_bytes());
        forwarded_head.extend_from_slice(b"\r\n");
    }
    forwarded_head.extend_from_slice(b"\r\n");
    Ok(HttpRequest {
        target,
        kind: HttpRequestKind::Forward,
        forwarded_head,
        buffered_body,
    })
}

fn ensure_http_version(version: &str) -> Result<()> {
    if matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        Ok(())
    } else {
        bail!("unsupported HTTP proxy protocol version {version}")
    }
}

fn ensure_no_extra_request_line_parts<'a>(mut parts: impl Iterator<Item = &'a str>) -> Result<()> {
    if parts.next().is_none() {
        Ok(())
    } else {
        bail!("invalid HTTP proxy request line")
    }
}

fn parse_authority(authority: &str) -> Result<ProxyTarget> {
    if let Ok(address) = authority.parse::<SocketAddr>() {
        return Ok(ProxyTarget {
            host: TargetHost::Address(address.ip()),
            port: address.port(),
        });
    }
    let (host, port) = authority
        .rsplit_once(':')
        .context("HTTP CONNECT target has no port")?;
    let port = port
        .parse::<u16>()
        .context("HTTP CONNECT target has an invalid port")?;
    if host.is_empty() || host.contains(':') {
        bail!("HTTP CONNECT target has an invalid host");
    }
    Ok(ProxyTarget {
        host: TargetHost::Domain(host.to_owned()),
        port,
    })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

async fn send_http_error(client: &mut TcpStream) -> io::Result<()> {
    client
        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\nContent-Length: 0\r\n\r\n")
        .await
}

async fn connect_direct(target: &ProxyTarget) -> io::Result<TcpStream> {
    match &target.host {
        TargetHost::Address(address) => {
            TcpStream::connect(SocketAddr::new(*address, target.port)).await
        }
        TargetHost::Domain(domain) => TcpStream::connect((domain.as_str(), target.port)).await,
    }
}

async fn resolve_overlay_target(
    net: &Arc<Net>,
    target: &ProxyTarget,
    dns_server: Option<SocketAddr>,
) -> Option<SocketAddr> {
    match &target.host {
        TargetHost::Address(IpAddr::V6(address)) if is_overlay_address(*address) => {
            return Some(SocketAddr::V6(SocketAddrV6::new(
                *address,
                target.port,
                0,
                0,
            )));
        }
        TargetHost::Address(_) => return None,
        TargetHost::Domain(_) => {}
    }

    let TargetHost::Domain(domain) = &target.host else {
        return None;
    };
    if let Some(server) = dns_server {
        match resolve_with_overlay_dns(net, domain, server).await {
            Ok(address) if is_overlay_address(address) => {
                return Some(SocketAddr::V6(SocketAddrV6::new(
                    address,
                    target.port,
                    0,
                    0,
                )));
            }
            Ok(_) => {}
            Err(error) => tracing::debug!(%error, %domain, "overlay DNS did not resolve target"),
        }
    }

    match tokio::net::lookup_host((domain.as_str(), target.port)).await {
        Ok(mut addresses) => addresses.find(|address| match address.ip() {
            IpAddr::V6(address) => is_overlay_address(address),
            IpAddr::V4(_) => false,
        }),
        Err(error) => {
            tracing::debug!(%error, %domain, "system DNS did not resolve target");
            None
        }
    }
}

fn is_overlay_address(address: Ipv6Addr) -> bool {
    address.octets()[0] & 0xfe == 0x02
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

    #[test]
    fn yggdrasil_range_is_selected_for_overlay_routing() {
        assert!(is_overlay_address("200:db8::1".parse().unwrap()));
        assert!(is_overlay_address("3ff:ffff::1".parse().unwrap()));
        assert!(!is_overlay_address("400::1".parse().unwrap()));
        assert!(!is_overlay_address("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn http_connect_accepts_domain_and_ipv6_authorities() {
        assert_eq!(
            parse_authority("example.test:443").unwrap(),
            ProxyTarget {
                host: TargetHost::Domain("example.test".to_owned()),
                port: 443,
            }
        );
        assert_eq!(
            parse_authority("[200:db8::1]:8443").unwrap(),
            ProxyTarget {
                host: TargetHost::Address("200:db8::1".parse().unwrap()),
                port: 8443,
            }
        );
    }

    #[test]
    fn http_forward_request_uses_origin_form_and_keeps_buffered_body() {
        let bytes = b"POST http://example.test:8080/path?q=1 HTTP/1.1\r\nHost: example.test\r\nProxy-Connection: keep-alive\r\nContent-Length: 4\r\n\r\ntest".to_vec();
        let header_end = find_header_end(&bytes).unwrap();
        let request = parse_http_request(bytes, header_end).unwrap();

        assert_eq!(request.kind, HttpRequestKind::Forward);
        assert_eq!(request.target.port, 8080);
        assert_eq!(request.buffered_body, b"test");
        let head = String::from_utf8(request.forwarded_head).unwrap();
        assert!(head.starts_with("POST /path?q=1 HTTP/1.1\r\n"));
        assert!(!head.to_ascii_lowercase().contains("proxy-connection"));
    }
}
