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

fn proxy_config() -> StoredConfig {
    let mut config = StoredConfig::default();
    config.drosophila.mode = ConnectionMode::Proxy;
    config.drosophila.proxy_listen = "127.0.0.1:0".to_owned();
    config.yggdrasil.listen.clear();
    config.yggdrasil.peers.clear();
    config.yggdrasil.multicast_interfaces.clear();
    config
}

fn overlay_packet(source: Ipv6Addr, destination: Ipv6Addr, payload: &[u8]) -> Vec<u8> {
    let udp_len = u16::try_from(8 + payload.len()).unwrap();
    let mut packet = vec![0_u8; 40 + usize::from(udp_len)];
    packet[0] = 0x60;
    packet[4..6].copy_from_slice(&udp_len.to_be_bytes());
    packet[6] = 17;
    packet[7] = 64;
    packet[8..24].copy_from_slice(&source.octets());
    packet[24..40].copy_from_slice(&destination.octets());
    packet[40..42].copy_from_slice(&12345_u16.to_be_bytes());
    packet[42..44].copy_from_slice(&23456_u16.to_be_bytes());
    packet[44..46].copy_from_slice(&udp_len.to_be_bytes());
    packet[48..].copy_from_slice(payload);
    let mut sum = u32::from(udp_len) + 17;
    for bytes in packet[8..40].chunks_exact(2).chain(packet[40..].chunks(2)) {
        sum += u32::from(u16::from_be_bytes([bytes[0], *bytes.get(1).unwrap_or(&0)]));
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    let checksum = !(sum as u16);
    packet[46..48].copy_from_slice(&if checksum == 0 { 0xffff } else { checksum }.to_be_bytes());
    packet
}

#[tokio::test]
async fn localhost_nodes_exchange_ipv6_packets_over_rwc() {
    let first_config = proxy_config().yggdrasil;
    let second_config = proxy_config().yggdrasil;
    let first = Core::new(first_config.signing_key().unwrap(), first_config);
    let second = Core::new(second_config.signing_key().unwrap(), second_config);
    let first_rwc = ReadWriteCloser::new(first.clone(), first.mtu(), None);
    let second_rwc = ReadWriteCloser::new(second.clone(), second.mtu(), None);
    first.set_path_notify(first_rwc.clone());
    second.set_path_notify(second_rwc.clone());

    let result: anyhow::Result<()> = tokio::time::timeout(Duration::from_secs(30), async {
        first.init_links().await;
        second.init_links().await;

        let mut peer_uri = None;
        for _ in 0..10 {
            let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let uri = format!("tcp://{}", reservation.local_addr()?);
            drop(reservation);
            match first.listen(&uri).await {
                Ok(()) => {
                    peer_uri = Some(uri);
                    break;
                }
                Err(error) if error.contains("bind failed") => continue,
                Err(error) => anyhow::bail!("listener: {error}"),
            }
        }
        let peer_uri = peer_uri.ok_or_else(|| anyhow::anyhow!("could not reserve a listener port"))?;
        second.add_peer(&peer_uri).await.map_err(|error| anyhow::anyhow!(error))?;

        loop {
            let first_up = first.get_peers().await.iter().any(|peer| peer.up && peer.key == *second.public_key());
            let second_up = second.get_peers().await.iter().any(|peer| peer.up && peer.key == *first.public_key());
            if first_up && second_up {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        for key in first.get_peer_keys().await { first_rwc.update_key(key).await; }
        for key in second.get_peer_keys().await { second_rwc.update_key(key).await; }

        let first_addr = Ipv6Addr::from(first.address().0);
        let second_addr = Ipv6Addr::from(second.address().0);
        for (sender, receiver, packet) in [
            (&first_rwc, &second_rwc, overlay_packet(first_addr, second_addr, b"first to second")),
            (&second_rwc, &first_rwc, overlay_packet(second_addr, first_addr, b"second to first")),
        ] {
            let mut buffer = vec![0_u8; 65536];
            let mut retry = tokio::time::interval(Duration::from_millis(200));
            loop {
                tokio::select! {
                    _ = retry.tick() => {
                        let written = sender.write(&packet).await.map_err(|error| anyhow::anyhow!(error))?;
                        anyhow::ensure!(written == packet.len(), "short RWC write: {written}");
                    }
                    received = receiver.read(&mut buffer) => {
                        let received = received.map_err(|error| anyhow::anyhow!(error))?;
                        anyhow::ensure!(received == packet, "received packet differs from sent IPv6/UDP datagram");
                        break;
                    }
                }
            }
        }
        Ok(())
    }).await.map_err(|_| anyhow::anyhow!("timed out connecting nodes or exchanging packets")).and_then(|result| result);

    let (first_close, second_close) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(5), first.close()),
        tokio::time::timeout(Duration::from_secs(5), second.close()),
    );
    assert!(
        first_close.is_ok_and(|result| result.is_ok()),
        "first core did not close"
    );
    assert!(
        second_close.is_ok_and(|result| result.is_ok()),
        "second core did not close"
    );
    result.unwrap();
}

#[tokio::test]
async fn proxy_start_stop_can_be_repeated() {
    for _ in 0..2 {
        let node = RunningNode::start(proxy_config()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), node.close())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn pending_registrations_are_aborted_and_joined() {
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = wait.await;
        Ok(())
    });
    let mut pending = HashMap::new();
    pending.insert("peer".to_owned(), handle);
    abort_pending_peers(pending).await;
    assert!(release.send(()).is_err());
}

#[tokio::test]
async fn configured_peer_can_be_removed_and_added_again() {
    let mut config = proxy_config();
    let peer = "tcp://127.0.0.1:1";
    config.yggdrasil.peers.push(peer.to_owned());
    let node = RunningNode::start(config).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if node
                .core
                .get_peers()
                .await
                .iter()
                .any(|entry| entry.uri == peer)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        node.remove_peer(peer).await.unwrap();
        node.add_peer(peer).await.unwrap();
        node.remove_peer(peer).await.unwrap();
        node.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn removing_pending_peer_cancels_registration() {
    let node = RunningNode::start(proxy_config()).await.unwrap();
    let peer = "tcp://127.0.0.1:1";
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let core = node.core.clone();
    let handle = tokio::spawn(async move {
        let _ = wait.await;
        core.add_peer(peer).await
    });
    node.pending_peers
        .lock()
        .await
        .insert(peer.to_owned(), handle);
    assert!(node.add_peer(peer).await.is_err());
    node.remove_peer(peer).await.unwrap();
    assert!(release.send(()).is_err());
    node.add_peer(peer).await.unwrap();
    node.remove_peer(peer).await.unwrap();
    node.close().await;
}

#[tokio::test]
async fn failed_proxy_start_releases_listener() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = proxy_config();
    config.drosophila.proxy_listen = address.to_string();
    assert!(RunningNode::start(config).await.is_err());
    drop(listener);
    let _listener = tokio::net::TcpListener::bind(address).await.unwrap();
}
