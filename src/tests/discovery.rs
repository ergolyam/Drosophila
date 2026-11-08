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
