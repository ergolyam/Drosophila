use super::*;

#[tokio::test]
async fn proxy_bind_failure_does_not_start_bridge() {
    let config = crate::config::StoredConfig::default().yggdrasil;
    let core = yggdrasil::core::Core::new(config.signing_key().unwrap(), config);
    let rwc = ReadWriteCloser::new(core.clone(), 1500, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen = listener.local_addr().unwrap();

    let error =
        match UserspaceProxy::start(rwc.clone(), Ipv6Addr::LOCALHOST, 1500, listen, None).await {
            Ok(proxy) => {
                proxy.close().await;
                panic!("binding to an occupied port unexpectedly succeeded");
            }
            Err(error) => error,
        };
    assert!(error.to_string().contains("binding local proxy"));
    tokio::task::yield_now().await;
    assert_eq!(Arc::strong_count(&rwc), 1, "bridge retained the RWC");
    core.close().await.unwrap();
}

#[tokio::test]
async fn rwc_writer_continues_after_packet_error_and_stops_on_close() {
    let (tx, rx) = mpsc::channel(2);
    let (written_tx, mut written_rx) = mpsc::unbounded_channel();
    let writer = tokio::spawn(write_rwc_packets(rx, move |packet| {
        let written_tx = written_tx.clone();
        async move {
            written_tx.send(packet.clone()).unwrap();
            if packet == [1] {
                Err("invalid packet".to_owned())
            } else {
                Ok(packet.len())
            }
        }
    }));

    tx.send(vec![1]).await.unwrap();
    tx.send(vec![2]).await.unwrap();
    drop(tx);
    tokio::time::timeout(Duration::from_secs(2), writer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(written_rx.recv().await.unwrap(), [1]);
    assert_eq!(written_rx.recv().await.unwrap(), [2]);
    assert!(written_rx.recv().await.is_none());
}

#[tokio::test]
async fn rwc_read_forwards_exact_slice_and_owns_packet() {
    let (tx, mut rx) = mpsc::channel(2);
    let mut buffer = vec![0xaa; RWC_READ_BUFFER_SIZE];
    buffer[10..14].copy_from_slice(&[1, 2, 3, 4]);

    assert!(forward_rwc_read(Ok(&buffer[10..14]), &tx).await);
    buffer[10..14].fill(0);
    assert_eq!(rx.recv().await.unwrap().unwrap(), [1, 2, 3, 4]);

    assert!(forward_rwc_read(Ok(&buffer), &tx).await);
    assert_eq!(rx.recv().await.unwrap().unwrap(), buffer);
    assert_eq!(RWC_READ_BUFFER_SIZE, 65_536);
}

#[tokio::test]
async fn rwc_read_forwards_error_and_stops() {
    let (tx, mut rx) = mpsc::channel(1);
    assert!(!forward_rwc_read(Err("read failed".to_owned()), &tx).await);
    let error = rx.recv().await.unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert_eq!(error.to_string(), "read failed");
}

#[tokio::test]
async fn rwc_read_stops_when_receiver_is_closed() {
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    assert!(!forward_rwc_read(Ok(&[1, 2, 3]), &tx).await);
    assert!(!forward_rwc_read(Err("read failed".to_owned()), &tx).await);
}

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
