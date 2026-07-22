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
