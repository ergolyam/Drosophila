use std::collections::HashMap;

use anyhow::{Result, bail};
use tokio::sync::mpsc;

use crate::config::StoredConfig;

// These protocol types keep the backend's event loop identical while the
// privileged worker implementation and all TUN dependencies remain uncompiled.
#[allow(dead_code)]
pub(crate) enum WorkerEvent {
    Started { address: String, subnet: String },
    StartFailed { message: String },
    OperationFailed { message: String },
    PeerStatus { statuses: HashMap<String, bool> },
}

pub(crate) struct RemoteEnvelope {
    pub(crate) session: u64,
    pub(crate) event: RemoteEvent,
}

#[allow(dead_code)]
pub(crate) enum RemoteEvent {
    Worker(WorkerEvent),
    Disconnected { error: Option<String> },
}

pub(crate) struct PrivilegedNode;

impl PrivilegedNode {
    pub(crate) async fn launch(
        _config: StoredConfig,
        _session: u64,
        _events: mpsc::UnboundedSender<RemoteEnvelope>,
    ) -> Result<Self> {
        std::future::ready(()).await;
        bail!("this build does not include TUN support")
    }

    pub(crate) fn add_peer(&self, _peer: String) {
        let _ = self;
    }

    pub(crate) fn remove_peer(&self, _peer: String) {
        let _ = self;
    }

    pub(crate) async fn close(self) {
        std::future::ready(()).await;
    }
}
