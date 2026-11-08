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
fn removing_request_keeps_other_cached_protocols() {
    let mut cache = DiscoveryCache::default();
    cache.insert(&protocols(&["tcp", "tls"]), Vec::new());
    cache.insert(&protocols(&["ws"]), Vec::new());

    cache.remove(&protocols(&["tls", "tcp"]));

    assert!(cache.get(&protocols(&["tcp", "tls"])).is_none());
    assert!(cache.get(&protocols(&["ws"])).is_some());
}
