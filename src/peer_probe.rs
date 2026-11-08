use std::io::{self, BufRead};
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use yggdrasil::{config::Config, core::Core, links::PeerEvent};

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn run() -> bool {
    let mut uri = String::new();
    if io::stdin().lock().read_line(&mut uri).is_err() {
        return false;
    }
    let uri = uri.trim_end_matches(['\r', '\n']);
    if uri.is_empty() {
        return false;
    }

    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    let success = runtime.block_on(probe(uri, PROBE_TIMEOUT));
    runtime.shutdown_timeout(SHUTDOWN_TIMEOUT);
    success
}

async fn probe(uri: &str, timeout: Duration) -> bool {
    let config = Config::generate();
    let Ok(key) = config.signing_key() else {
        return false;
    };
    let core = Core::new(key, config);
    core.init_links().await;
    let mut events = core.subscribe_peer_events();

    let success = tokio::time::timeout(timeout, async {
        if core.add_peer(uri).await.is_err() {
            return false;
        }
        loop {
            match events.recv().await {
                Ok(PeerEvent::Connected {
                    uri: connected_uri,
                    inbound: false,
                    ..
                }) if connected_uri == uri => return true,
                Ok(_) | Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, core.close()).await;
    success
}

#[cfg(test)]
#[path = "tests/peer_probe.rs"]
mod tests;
