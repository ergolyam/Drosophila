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
fn http_forward_request_uses_origin_form_and_disables_connection_reuse() {
    let bytes = b"POST http://example.test:8080/path?q=1 HTTP/1.1\r\nHost: example.test\r\nConnection: keep-alive\r\nProxy-Connection: keep-alive\r\nContent-Length: 4\r\n\r\ntest".to_vec();
    let header_end = find_header_end(&bytes).unwrap();
    let request = parse_http_request(bytes, header_end).unwrap();

    assert_eq!(request.kind, HttpRequestKind::Forward);
    assert_eq!(request.target.port, 8080);
    assert_eq!(request.buffered_body, b"test");
    let head = String::from_utf8(request.forwarded_head).unwrap();
    assert!(head.starts_with("POST /path?q=1 HTTP/1.1\r\n"));
    assert!(!head.to_ascii_lowercase().contains("proxy-connection"));
    assert!(!head.to_ascii_lowercase().contains("connection: keep-alive"));
    assert!(head.ends_with("Connection: close\r\n\r\n"));
}
