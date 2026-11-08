use super::*;

fn protocols(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn peer_builder_percent_encodes_secrets() {
    let peer = build_peer_uri("tls", "example.com:443", "front.example", "a b").unwrap();
    assert!(peer.contains("sni=front.example"));
    assert!(peer.contains("password=a+b"));
}

#[test]
fn discovered_peer_keeps_verified_uri() {
    let peer = DiscoveredPeer {
        address: "wss://example.com/yggdrasil?key=abc&password=secret".to_owned(),
        protocol: "wss".to_owned(),
        host: "example.com".to_owned(),
        port: 443,
        country: "test".to_owned(),
        ping_ms: None,
    };
    assert_eq!(
        build_discovered_peer_uri(Some(&peer), "wss", "example.com:443", "", "secret"),
        Some(peer.address.clone())
    );
    let edited =
        build_discovered_peer_uri(Some(&peer), "wss", "example.com:443", "", "new").unwrap();
    assert!(edited.contains("/yggdrasil?"));
    assert!(edited.contains("key=abc"));
    assert!(edited.contains("password=new"));
}

fn discovered(address: &str) -> DiscoveredPeer {
    DiscoveredPeer {
        address: address.to_owned(),
        protocol: "tcp".to_owned(),
        host: "example.com".to_owned(),
        port: 1234,
        country: "test".to_owned(),
        ping_ms: None,
    }
}

#[test]
fn progressive_and_final_peers_are_deduplicated_by_address() {
    let mut peers = Vec::new();
    assert!(merge_discovered_peers(&mut peers, [discovered("tcp://a")]));
    assert!(!merge_discovered_peers(&mut peers, [discovered("tcp://a")]));
    assert!(merge_discovered_peers(
        &mut peers,
        [
            discovered("tcp://a"),
            discovered("tcp://b"),
            discovered("tcp://b")
        ],
    ));
    assert_eq!(peers.len(), 2);
    assert_eq!(peers[0].address, "tcp://a");
    assert_eq!(peers[1].address, "tcp://b");
    let mut updated = discovered("tcp://a");
    updated.ping_ms = Some(25);
    assert!(merge_discovered_peers(&mut peers, [updated]));
    assert_eq!(peers.len(), 2);
    assert_eq!(peers[0].ping_ms, Some(25));
}

#[test]
fn progressive_updates_are_coalesced_without_losing_changes() {
    let mut peers = DiscoveryPeers::default();
    for i in 0..1_000 {
        peers.merge([discovered(&format!("tcp://{i}"))]);
    }
    let mut updated = discovered("tcp://10");
    updated.ping_ms = Some(25);
    peers.merge([updated.clone()]);
    assert_eq!(peers.entries.len(), 1_000);
    assert_eq!(peers.entries[10], updated);
    assert!(peers.take_pending_refresh());
    assert!(!peers.take_pending_refresh());

    peers.merge([updated.clone()]);
    assert!(!peers.take_pending_refresh());
    updated.ping_ms = Some(50);
    peers.merge([updated]);
    assert!(peers.take_pending_refresh());
    assert_eq!(peers.entries[10].ping_ms, Some(50));
}

#[test]
fn immediate_refresh_and_new_search_clear_pending_updates() {
    let mut peers = DiscoveryPeers::default();
    peers.merge([discovered("tcp://a")]);
    // An immediate sort or final-result refresh consumes the pending update.
    assert!(peers.take_pending_refresh());
    assert!(!peers.take_pending_refresh());
    peers.merge([discovered("tcp://b")]);
    peers = DiscoveryPeers::default();
    assert!(peers.entries.is_empty());
    assert!(!peers.take_pending_refresh());
}

#[test]
fn discovery_status_distinguishes_search_error_and_empty_result() {
    assert_eq!(discovery_status(0, true, None), "Searching for peers…");
    assert!(discovery_status(1, true, None).contains("Shown 1 peers"));
    assert_eq!(discovery_status(0, false, None), "No peers available");
    let status = discovery_status(1, false, Some("offline"));
    assert!(status.contains("offline"));
    assert!(status.contains("Shown 1 peers"));
}

#[test]
fn ipv6_endpoint_is_bracketed() {
    assert_eq!(format_endpoint("2001:db8::1", 443), "[2001:db8::1]:443");
}

#[test]
fn protocol_order_does_not_affect_cache_lookup() {
    let mut cache = DiscoveryCache::default();
    cache.insert(&protocols(&["tcp", "tls"]), Vec::new());

    assert!(cache.get(&protocols(&["tls", "tcp"])).is_some());
}

#[test]
fn session_cache_preserves_partial_and_completed_searches() {
    let mut cache = DiscoveryCache::default();
    let tcp = protocols(&["tcp"]);
    let ws = protocols(&["ws"]);
    let peer = discovered("tcp://a");

    cache.insert_partial(&tcp, &[]);
    assert!(cache.get(&tcp).is_none());
    cache.insert_partial(&tcp, std::slice::from_ref(&peer));
    let partial = cache.get(&tcp).unwrap();
    assert!(!partial.complete);
    assert_eq!(partial.peers, vec![peer.clone()]);

    cache.insert(&tcp, vec![peer.clone()]);
    cache.insert(&ws, Vec::new());
    cache.insert_partial(&tcp, &[discovered("tcp://b")]);
    assert!(cache.get(&tcp).unwrap().complete);
    assert_eq!(
        cache.get(&tcp).unwrap().peers,
        vec![peer, discovered("tcp://b")]
    );
    assert!(cache.get(&ws).unwrap().complete);
    assert!(cache.get(&ws).unwrap().peers.is_empty());

    let partial_protocol = protocols(&["wss"]);
    cache.insert_partial(&partial_protocol, &[discovered("tcp://first")]);
    cache.insert_partial(&partial_protocol, &[discovered("tcp://second")]);
    assert_eq!(cache.get(&partial_protocol).unwrap().peers.len(), 2);
    cache.insert_partial(&ws, &[discovered("ws://new")]);
    assert_eq!(cache.get(&ws).unwrap().peers.len(), 1);

    cache.insert(&tcp, vec![discovered("tcp://updated")]);
    assert_eq!(cache.get(&tcp).unwrap().peers[0].address, "tcp://updated");
    assert!(DiscoveryCache::default().get(&tcp).is_none());

    cache.close_dialog(false);
    assert!(cache.get(&tcp).is_some());
    assert!(cache.get(&ws).is_some());
    cache.close_dialog(true);
    assert!(cache.get(&tcp).is_none());
    assert!(cache.get(&ws).is_none());
    assert!(cache.get(&partial_protocol).is_none());
    cache.insert_partial(&tcp, &[discovered("tcp://fresh")]);
    assert!(!cache.get(&tcp).unwrap().complete);
    assert_eq!(cache.get(&tcp).unwrap().peers[0].address, "tcp://fresh");
}
