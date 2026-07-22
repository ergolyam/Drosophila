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
