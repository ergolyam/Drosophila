use std::collections::HashMap;
use std::fmt::Write as _;
#[cfg(target_os = "linux")]
use std::io::Read as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use futures::{SinkExt, StreamExt};
use rand::RngCore;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use tokio::io::AsyncWriteExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_util::codec::{Framed, LinesCodec};

use crate::backend::RunningNode;
use crate::config::{ConnectionMode, StoredConfig, is_flatpak};

const WORKER_FLAG: &str = "--privileged-tun-worker";
const PROTOCOL_VERSION: u32 = 1;
const MAX_MESSAGE_LENGTH: usize = 1024 * 1024;
const ELEVATION_TIMEOUT: Duration = Duration::from_mins(2);
const AUTHENTICATION_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct WorkerArguments {
    endpoint: SocketAddr,
    token: String,
}

impl WorkerArguments {
    pub(crate) fn parse(arguments: &[String]) -> Option<Result<Self>> {
        if arguments.get(1).map(String::as_str) != Some(WORKER_FLAG) {
            return None;
        }
        Some(Self::parse_worker_arguments(arguments))
    }

    fn parse_worker_arguments(arguments: &[String]) -> Result<Self> {
        #[cfg(target_os = "linux")]
        ensure!(arguments.len() == 3, "invalid privileged worker arguments");
        #[cfg(not(target_os = "linux"))]
        ensure!(arguments.len() == 4, "invalid privileged worker arguments");

        #[cfg(target_os = "linux")]
        let token = {
            let mut token = String::new();
            std::io::stdin()
                .take(65)
                .read_to_string(&mut token)
                .context("reading the privileged worker token")?;
            token
        };
        #[cfg(not(target_os = "linux"))]
        let token = arguments[3].clone();

        Self::from_parts(&arguments[2], token)
    }

    fn from_parts(endpoint: &str, token: String) -> Result<Self> {
        let endpoint = endpoint
            .parse::<SocketAddr>()
            .context("invalid privileged worker endpoint")?;
        ensure!(
            endpoint.ip().is_loopback(),
            "the privileged worker endpoint must be on loopback"
        );
        ensure!(
            token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid privileged worker token"
        );
        Ok(Self { endpoint, token })
    }
}

#[derive(Deserialize, Serialize)]
struct WorkerHello {
    version: u32,
    token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
enum WorkerCommand {
    Start { config: Box<StoredConfig> },
    AddPeer { peer: String },
    RemovePeer { peer: String },
    Shutdown,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
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

pub(crate) enum RemoteEvent {
    Worker(WorkerEvent),
    Disconnected { error: Option<String> },
}

pub(crate) struct PrivilegedNode {
    commands: mpsc::UnboundedSender<WorkerCommand>,
    closed: oneshot::Receiver<()>,
    disconnect: Option<oneshot::Sender<()>>,
}

impl PrivilegedNode {
    pub(crate) async fn launch(
        config: StoredConfig,
        session: u64,
        events: mpsc::UnboundedSender<RemoteEnvelope>,
    ) -> Result<Self> {
        if is_flatpak() {
            bail!(
                "Flatpak cannot create a host TUN interface. Enable the SOCKS Proxy mode in Settings."
            );
        }

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .context("binding the privileged worker IPC listener")?;
        let endpoint = listener
            .local_addr()
            .context("reading the privileged worker IPC endpoint")?;
        let token = random_token();
        let executable = std::env::current_exe().context("locating the Drosophila executable")?;
        #[cfg(target_os = "linux")]
        let worker_arguments = vec![WORKER_FLAG.to_owned(), endpoint.to_string()];
        #[cfg(not(target_os = "linux"))]
        let worker_arguments = vec![WORKER_FLAG.to_owned(), endpoint.to_string(), token.clone()];
        #[cfg(windows)]
        let launcher_exit = launch_elevated(&executable, &worker_arguments, &token);
        #[cfg(not(windows))]
        let launcher_exit = launch_elevated(&executable, &worker_arguments, &token)?;
        let framed = accept_authenticated(listener, &token, launcher_exit).await?;

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (closed_tx, closed_rx) = oneshot::channel();
        let (disconnect_tx, disconnect_rx) = oneshot::channel();
        tokio::spawn(parent_connection(
            framed,
            command_rx,
            events,
            session,
            closed_tx,
            disconnect_rx,
        ));

        command_tx
            .send(WorkerCommand::Start {
                config: Box::new(config),
            })
            .map_err(|_| anyhow!("the privileged TUN worker disconnected during startup"))?;

        Ok(Self {
            commands: command_tx,
            closed: closed_rx,
            disconnect: Some(disconnect_tx),
        })
    }

    pub(crate) fn add_peer(&self, peer: String) {
        let _ = self.commands.send(WorkerCommand::AddPeer { peer });
    }

    pub(crate) fn remove_peer(&self, peer: String) {
        let _ = self.commands.send(WorkerCommand::RemovePeer { peer });
    }

    pub(crate) async fn close(mut self) {
        let _ = self.commands.send(WorkerCommand::Shutdown);
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut self.closed)
            .await
            .is_err()
            && let Some(disconnect) = self.disconnect.take()
        {
            let _ = disconnect.send(());
        }
    }
}

pub(crate) fn run_worker(arguments: WorkerArguments) -> Result<()> {
    #[cfg(target_os = "linux")]
    restrict_linux_worker_privileges()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("drosophila-tun-worker")
        .build()
        .context("building the privileged worker runtime")?;
    runtime.block_on(worker_loop(arguments))
}

async fn worker_loop(arguments: WorkerArguments) -> Result<()> {
    let stream = TcpStream::connect(arguments.endpoint)
        .await
        .context("connecting to the Drosophila IPC listener")?;
    let framed = Framed::new(stream, LinesCodec::new_with_max_length(MAX_MESSAGE_LENGTH));
    let (mut sink, mut source) = framed.split();
    send_json(
        &mut sink,
        &WorkerHello {
            version: PROTOCOL_VERSION,
            token: arguments.token,
        },
    )
    .await?;

    let first = source
        .next()
        .await
        .context("the GUI disconnected before starting TUN")?
        .context("reading the TUN start command")?;
    let WorkerCommand::Start { mut config } =
        serde_json::from_str::<WorkerCommand>(&first).context("decoding the TUN start command")?
    else {
        bail!("the first privileged worker command was not start");
    };
    config.drosophila.mode = ConnectionMode::Tun;

    let node = match RunningNode::start(*config).await {
        Ok(node) => node,
        Err(error) => {
            send_json(
                &mut sink,
                &WorkerEvent::StartFailed {
                    message: format!("{error:#}"),
                },
            )
            .await?;
            return Ok(());
        }
    };

    let result = async {
        send_json(
            &mut sink,
            &WorkerEvent::Started {
                address: node.address().to_owned(),
                subnet: node.subnet().to_owned(),
            },
        )
        .await?;

        let mut status_timer = tokio::time::interval(Duration::from_secs(1));
        status_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                line = source.next() => {
                    let Some(line) = line else { break };
                    let line = line.context("reading a privileged worker command")?;
                    let command = serde_json::from_str::<WorkerCommand>(&line)
                        .context("decoding a privileged worker command")?;
                    match command {
                        WorkerCommand::Start { .. } => {
                            send_json(&mut sink, &WorkerEvent::OperationFailed {
                                message: "the privileged worker is already running TUN".to_owned(),
                            }).await?;
                        }
                        WorkerCommand::AddPeer { peer } => {
                            if let Err(error) = node.add_peer(&peer).await {
                                send_json(&mut sink, &WorkerEvent::OperationFailed {
                                    message: format!("Failed to add peer: {error}"),
                                }).await?;
                            }
                        }
                        WorkerCommand::RemovePeer { peer } => {
                            if let Err(error) = node.remove_peer(&peer).await {
                                send_json(&mut sink, &WorkerEvent::OperationFailed {
                                    message: format!("Failed to remove peer: {error}"),
                                }).await?;
                            }
                        }
                        WorkerCommand::Shutdown => break,
                    }
                }
                _ = status_timer.tick() => {
                    send_json(&mut sink, &WorkerEvent::PeerStatus {
                        statuses: node.peer_status().await,
                    }).await?;
                }
            }
        }

        Ok(())
    }
    .await;

    node.close().await;
    result
}

async fn parent_connection(
    framed: Framed<TcpStream, LinesCodec>,
    mut commands: mpsc::UnboundedReceiver<WorkerCommand>,
    events: mpsc::UnboundedSender<RemoteEnvelope>,
    session: u64,
    closed: oneshot::Sender<()>,
    mut disconnect: oneshot::Receiver<()>,
) {
    let (mut sink, mut source) = framed.split();
    let mut shutdown_sent = false;
    let mut startup_failure_reported = false;
    let mut connection_error = None;

    loop {
        tokio::select! {
            command = commands.recv(), if !shutdown_sent => {
                let command = command.unwrap_or(WorkerCommand::Shutdown);
                shutdown_sent = matches!(command, WorkerCommand::Shutdown);
                if let Err(error) = send_json(&mut sink, &command).await {
                    connection_error = Some(format!("sending a command to the privileged TUN worker: {error:#}"));
                    break;
                }
            }
            line = source.next() => {
                let Some(line) = line else { break };
                let line = match line {
                    Ok(line) => line,
                    Err(error) => {
                        connection_error = Some(format!("reading from the privileged TUN worker: {error}"));
                        break;
                    }
                };
                match serde_json::from_str::<WorkerEvent>(&line) {
                    Ok(event) => {
                        startup_failure_reported = matches!(event, WorkerEvent::StartFailed { .. });
                        let _ = events.send(RemoteEnvelope {
                            session,
                            event: RemoteEvent::Worker(event),
                        });
                    }
                    Err(error) => {
                        connection_error = Some(format!("decoding a privileged TUN worker event: {error}"));
                        break;
                    }
                }
            }
            _ = &mut disconnect => {
                shutdown_sent = true;
                break;
            }
        }
    }

    let error = if shutdown_sent || startup_failure_reported {
        None
    } else {
        Some(
            connection_error
                .unwrap_or_else(|| "the privileged TUN worker exited unexpectedly".to_owned()),
        )
    };
    let _ = events.send(RemoteEnvelope {
        session,
        event: RemoteEvent::Disconnected { error },
    });
    let _ = closed.send(());
}

async fn accept_authenticated(
    listener: TcpListener,
    token: &str,
    mut launcher_exit: oneshot::Receiver<String>,
) -> Result<Framed<TcpStream, LinesCodec>> {
    let elevation_timeout = tokio::time::sleep(ELEVATION_TIMEOUT);
    tokio::pin!(elevation_timeout);

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.context("accepting the privileged worker IPC connection")?;
                let mut framed = Framed::new(stream, LinesCodec::new_with_max_length(MAX_MESSAGE_LENGTH));
                let hello = tokio::time::timeout(AUTHENTICATION_TIMEOUT, framed.next()).await;
                let Ok(Some(Ok(line))) = hello else { continue };
                let Ok(hello) = serde_json::from_str::<WorkerHello>(&line) else { continue };
                if hello.version == PROTOCOL_VERSION && hello.token == token {
                    return Ok(framed);
                }
            }
            exit = &mut launcher_exit => {
                bail!(exit.unwrap_or_else(|_| "the privilege launcher stopped unexpectedly".to_owned()));
            }
            () = &mut elevation_timeout => {
                bail!("timed out waiting for administrator authorization");
            }
        }
    }
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut token, byte| {
            write!(token, "{byte:02x}").expect("writing to a String cannot fail");
            token
        })
}

async fn send_json<T>(
    sink: &mut futures::stream::SplitSink<Framed<TcpStream, LinesCodec>, String>,
    value: &T,
) -> Result<()>
where
    T: Serialize,
{
    let message = serde_json::to_string(value).context("encoding an IPC message")?;
    sink.send(message).await.context("sending an IPC message")
}

#[cfg(target_os = "linux")]
fn launch_elevated(
    executable: &Path,
    arguments: &[String],
    token: &str,
) -> Result<oneshot::Receiver<String>> {
    let mut command = tokio::process::Command::new("pkexec");
    command
        .arg("--disable-internal-agent")
        .arg(executable)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    let mut child = command
        .spawn()
        .context("starting PolicyKit pkexec (is pkexec installed?)")?;
    let mut stdin = child
        .stdin
        .take()
        .context("opening the PolicyKit worker token channel")?;
    let token = token.as_bytes().to_vec();
    tokio::spawn(async move {
        let _ = stdin.write_all(&token).await;
        let _ = stdin.shutdown().await;
    });
    let (sender, receiver) = oneshot::channel();
    tokio::spawn(async move {
        let message = match child.wait().await {
            Ok(status) => format!("the PolicyKit worker exited before connecting ({status})"),
            Err(error) => format!("waiting for the PolicyKit worker failed: {error}"),
        };
        let _ = sender.send(message);
    });
    Ok(receiver)
}

#[cfg(windows)]
fn launch_elevated(
    executable: &Path,
    arguments: &[String],
    _token: &str,
) -> oneshot::Receiver<String> {
    let executable = PathBuf::from(executable);
    let arguments = arguments.to_vec();
    let (sender, receiver) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let mut command = runas::Command::new(executable);
        command.args(&arguments).gui(true).show(false);
        let message = match command.status() {
            Ok(status) => format!("the UAC worker exited before connecting ({status})"),
            Err(error) => format!("starting the UAC worker failed: {error}"),
        };
        let _ = sender.send(message);
    });
    receiver
}

#[cfg(not(any(target_os = "linux", windows)))]
fn launch_elevated(
    _executable: &Path,
    _arguments: &[String],
    _token: &str,
) -> Result<oneshot::Receiver<String>> {
    bail!("dynamic TUN authorization is only supported on Linux and Windows")
}

#[cfg(target_os = "linux")]
fn restrict_linux_worker_privileges() -> Result<()> {
    use capctl::caps::{Cap, CapState, bounding, cap_set_ids};
    use nix::unistd::{Uid, User};

    let invoking_uid = std::env::var("PKEXEC_UID")
        .context("the privileged worker must be launched by PolicyKit pkexec")?
        .parse::<u32>()
        .context("PKEXEC_UID is not a valid user ID")?;
    ensure!(
        Uid::effective().is_root(),
        "PolicyKit did not start the TUN worker as root"
    );
    let user = User::from_uid(Uid::from_raw(invoking_uid))
        .context("looking up the invoking user")?
        .context("the invoking user no longer exists")?;

    for capability in Cap::iter() {
        if capability != Cap::NET_ADMIN && bounding::read(capability) == Some(true) {
            bounding::drop(capability).map_err(|error| {
                anyhow!("dropping {capability:?} from the capability bound: {error}")
            })?;
        }
    }
    bounding::clear_unknown().map_err(|error| anyhow!("dropping unknown capabilities: {error}"))?;
    cap_set_ids(Some(invoking_uid), Some(user.gid.as_raw()), Some(&[]))
        .map_err(|error| anyhow!("dropping root user and group IDs: {error}"))?;

    CapState {
        effective: capctl::capset!(Cap::NET_ADMIN),
        permitted: capctl::capset!(Cap::NET_ADMIN),
        inheritable: capctl::capset!(),
    }
    .set_current()
    .map_err(|error| anyhow!("restricting the worker to CAP_NET_ADMIN: {error}"))?;

    ensure!(
        Uid::effective().as_raw() == invoking_uid,
        "the TUN worker failed to drop its root user ID"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_arguments_require_loopback_and_strong_token() {
        let token = "ab".repeat(32);
        let parsed = WorkerArguments::from_parts("127.0.0.1:1234", token.clone()).unwrap();
        assert_eq!(parsed.endpoint, "127.0.0.1:1234".parse().unwrap());
        assert_eq!(parsed.token, token);

        assert!(WorkerArguments::from_parts("192.0.2.1:1234", "ab".repeat(32)).is_err());
    }

    #[test]
    fn protocol_rejects_unknown_commands() {
        assert!(serde_json::from_str::<WorkerCommand>(r#"{"command":"shell"}"#).is_err());
    }
}
