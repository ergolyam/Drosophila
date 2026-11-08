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
    assert_eq!(peers[0].ping_ms, None);
}

#[test]
fn candidates_reject_control_characters_in_uri() {
    let nodes: PublicNodes =
        serde_json::from_str(r#"{"DE.md":{"tcp://example.com:1234\n?key=abc":{"up":true}}}"#)
            .unwrap();
    assert!(build_candidates(nodes, &["tcp".to_owned()]).is_empty());
}

#[test]
fn parses_icmp_reply_times_without_using_summary() {
    assert_eq!(
        parse_ping_output("64 bytes from 127.0.0.1: time=12.34 ms"),
        Some(13)
    );
    assert_eq!(
        parse_ping_output("Reply from 127.0.0.1: time<1ms TTL=128"),
        Some(1)
    );
    assert_eq!(
        parse_ping_output("64 bytes from ::1: time=0.050 ms"),
        Some(1)
    );
    assert_eq!(parse_ping_output("round-trip min/avg/max = 1/2/3 ms"), None);
    assert_eq!(parse_ping_output("request timed out"), None);
    assert_eq!(parse_ping_output("time=12 seconds"), None);
}

#[tokio::test]
async fn reports_peers_before_slow_probes_and_ping_finish() {
    let peer = |address: &str| DiscoveredPeer {
        address: address.to_owned(),
        protocol: "tcp".to_owned(),
        host: "localhost".to_owned(),
        port: 1234,
        country: "test".to_owned(),
        ping_ms: None,
    };
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let (release_ping, ping_gate) = tokio::sync::oneshot::channel::<()>();
    let ping_gate = std::sync::Arc::new(tokio::sync::Mutex::new(Some(ping_gate)));
    let task = tokio::spawn(check_candidates(
        vec![peer("fast"), peer("slow")],
        |peer| async move {
            if peer.address == "slow" {
                std::future::pending::<()>().await;
            }
            Ok(Some(peer))
        },
        move |_| {
            let gate = ping_gate.clone();
            async move {
                if let Some(gate) = gate.lock().await.take() {
                    let _ = gate.await;
                }
                Some(42)
            }
        },
        move |peer| {
            let _ = events.send(peer);
        },
    ));
    let first = tokio::time::timeout(Duration::from_secs(2), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.address, "fast");
    assert_eq!(first.ping_ms, None);
    release_ping.send(()).unwrap();
    let updated = tokio::time::timeout(Duration::from_secs(2), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.address, "fast");
    assert_eq!(updated.ping_ms, Some(42));
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn saturated_pings_do_not_block_later_probe_results() {
    let peers = (0..70)
        .map(|number| DiscoveredPeer {
            address: number.to_string(),
            protocol: "tcp".to_owned(),
            host: "localhost".to_owned(),
            port: 1234,
            country: "test".to_owned(),
            ping_ms: None,
        })
        .collect();
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(check_candidates(
        peers,
        |peer| async move { Ok(Some(peer)) },
        |_| async { std::future::pending::<Option<u64>>().await },
        move |peer| {
            let _ = events.send(peer);
        },
    ));
    for expected in 0..70 {
        let peer = tokio::time::timeout(Duration::from_secs(2), received.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(peer.address, expected.to_string());
    }
    assert!(!task.is_finished());
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn probes_localhost_without_using_catalog_latency() {
    let peer = DiscoveredPeer {
        address: "quic://127.0.0.1:1234".to_owned(),
        protocol: "quic".to_owned(),
        host: "127.0.0.1".to_owned(),
        port: 1234,
        country: "test".to_owned(),
        ping_ms: Some(u64::MAX),
    };
    let direct = ping_address("127.0.0.1".parse().unwrap()).await;
    let result = ping_peer(&peer).await;
    assert_ne!(result, Some(u64::MAX));
    if direct.is_some() {
        assert!(result.is_some());
    }
}
