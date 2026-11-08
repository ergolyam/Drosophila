use super::*;
use tokio::net::TcpListener;

#[tokio::test]
async fn connects_to_yggdrasil_listener() {
    let config = Config::generate();
    let listener = Core::new(config.signing_key().unwrap(), config);
    listener.init_links().await;
    let mut uri = None;
    for _ in 0..10 {
        let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let candidate = format!("tcp://{}", reservation.local_addr().unwrap());
        drop(reservation);
        if listener.listen(&candidate).await.is_ok() {
            uri = Some(candidate);
            break;
        }
    }
    let uri = format!("{}?priority=1", uri.expect("could not bind listener"));
    let success = probe(&uri, Duration::from_secs(5)).await;
    let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, listener.close()).await;
    assert!(success);
}

#[tokio::test]
async fn open_non_yggdrasil_server_is_not_success() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("tcp://{}", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        let _ = accepted_tx.send(());
        std::future::pending::<()>().await;
    });
    let success = probe(&uri, Duration::from_millis(500)).await;
    let accepted = tokio::time::timeout(Duration::from_secs(1), accepted_rx).await;
    server.abort();
    assert!(matches!(accepted, Ok(Ok(()))));
    assert!(!success);
}

#[tokio::test]
async fn closed_port_is_not_success() {
    let reservation = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let uri = format!("tcp://{}", reservation.local_addr().unwrap());
    drop(reservation);
    assert!(!probe(&uri, Duration::from_millis(500)).await);
}

#[tokio::test]
async fn invalid_uri_options_are_not_discarded() {
    assert!(!probe("tcp://127.0.0.1:1?priority=invalid", Duration::from_secs(1)).await);
}
