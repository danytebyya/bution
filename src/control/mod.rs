//! Encrypted peer control server, pairing workflow, and RPC worker commands.

use crate::PROTOCOL_VERSION;
use crate::cluster::{
    ControlMessage, NodeStatus, NodeSummary, PairDecision, PairRequest, PairResponse,
};
use crate::hardware::HardwareProfile;
use crate::llama::{LlamaBinaries, WorkerConfig};
use crate::locale::text;
use crate::network::{BenchmarkServer, NetworkInterface, interfaces};
use crate::processes::{ProcessKind, ProcessManager};
use crate::security::{NoiseChannel, NoiseIdentity, pairing_code, validate_distinct_identity};
use crate::storage::{AppPaths, Settings, TrustedPeer};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

pub enum ControlEvent {
    PairingRequested {
        request: PairRequest,
        remote_address: SocketAddr,
        code: String,
        response: oneshot::Sender<PairDecision>,
    },
    WorkerStarted {
        remote_address: SocketAddr,
    },
    WorkerStopped {
        remote_address: SocketAddr,
    },
    ConnectionError {
        message: String,
        detail: String,
    },
}

#[derive(Clone)]
struct ServerContext {
    settings: Arc<Mutex<Settings>>,
    paths: AppPaths,
    identity: NoiseIdentity,
    binaries: Option<LlamaBinaries>,
    hardware: HardwareProfile,
    processes: Arc<Mutex<ProcessManager>>,
    network_benchmark: Arc<Mutex<Option<BenchmarkServer>>>,
    events: mpsc::Sender<ControlEvent>,
}

pub struct ControlServer {
    local_addr: SocketAddr,
    events: mpsc::Receiver<ControlEvent>,
    processes: Arc<Mutex<ProcessManager>>,
    network_benchmark: Arc<Mutex<Option<BenchmarkServer>>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl ControlServer {
    pub async fn start(
        bind_address: SocketAddr,
        settings: Arc<Mutex<Settings>>,
        paths: AppPaths,
        identity: NoiseIdentity,
        binaries: Option<LlamaBinaries>,
        hardware: HardwareProfile,
    ) -> Result<Self> {
        let listener = TcpListener::bind(bind_address)
            .await
            .with_context(|| format!("could not bind control service to {bind_address}"))?;
        let local_addr = listener.local_addr()?;
        let (events_sender, events) = mpsc::channel(64);
        let (shutdown, mut shutdown_receiver) = oneshot::channel();
        let processes = Arc::new(Mutex::new(ProcessManager::default()));
        let network_benchmark = Arc::new(Mutex::new(None));
        let context = ServerContext {
            settings,
            paths,
            identity,
            binaries,
            hardware,
            processes: processes.clone(),
            network_benchmark: network_benchmark.clone(),
            events: events_sender.clone(),
        };
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_receiver => break,
                    accepted = listener.accept() => match accepted {
                        Ok((stream, remote)) => {
                            let context = context.clone();
                            tokio::spawn(async move {
                                if let Err(error) = handle_connection(stream, remote, context.clone()).await {
                                    let _ = context.events.send(ControlEvent::ConnectionError {
                                        message: text("Secure connection with a node was interrupted", "Защищённое соединение с узлом прервано").into(),
                                        detail: format!("{error:#}"),
                                    }).await;
                                }
                            });
                        }
                        Err(error) => {
                            let _ = events_sender.send(ControlEvent::ConnectionError {
                                message: text("Could not accept a node connection", "Не удалось принять подключение узла").into(),
                                detail: error.to_string(),
                            }).await;
                        }
                    }
                }
            }
        });
        Ok(Self {
            local_addr,
            events,
            processes,
            network_benchmark,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn next_event(&mut self) -> Option<ControlEvent> {
        self.events.recv().await
    }

    pub fn processes(&self) -> Arc<Mutex<ProcessManager>> {
        self.processes.clone()
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.processes.lock().await.stop_all().await;
        if let Some(server) = self.network_benchmark.lock().await.take() {
            server.stop().await;
        }
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub struct ControlClient {
    channel: NoiseChannel,
    pub remote_id: uuid::Uuid,
    pub remote_name: String,
}

impl ControlClient {
    pub async fn pair(
        address: SocketAddr,
        identity: &NoiseIdentity,
        local_settings: &Settings,
    ) -> Result<Self> {
        let mut channel = NoiseChannel::connect(address, identity, Duration::from_secs(5)).await?;
        let remote_key = channel.remote_public_key().to_owned();
        channel
            .send(&ControlMessage::PairRequest(PairRequest {
                node_id: local_settings.node_id,
                node_name: local_settings.node_name.clone(),
                public_key: identity.public_key(),
                protocol_version: PROTOCOL_VERSION,
            }))
            .await?;
        let response = match channel.receive().await? {
            ControlMessage::PairResponse(response) => response,
            ControlMessage::Error { message, .. } => bail!("{message}"),
            _ => bail!("peer did not return a pairing decision"),
        };
        if response.public_key != remote_key || response.decision != PairDecision::Accept {
            bail!("pairing was rejected or the peer identity changed");
        }
        Ok(Self {
            channel,
            remote_id: response.node_id,
            remote_name: address.to_string(),
        })
    }

    pub fn remote_public_key(&self) -> &str {
        self.channel.remote_public_key()
    }

    pub async fn node_info(&mut self) -> Result<NodeSummary> {
        self.channel.send(&ControlMessage::GetNodeInfo).await?;
        match self.channel.receive().await? {
            ControlMessage::NodeInfo(info) => Ok(info),
            ControlMessage::Error { message, .. } => bail!("{message}"),
            _ => bail!("peer returned an unexpected node information response"),
        }
    }

    pub async fn start_worker(&mut self, bind_address: IpAddr, rpc_port: u16) -> Result<u16> {
        self.channel
            .send(&ControlMessage::StartWorker {
                bind_address: bind_address.to_string(),
                rpc_port,
            })
            .await?;
        match self.channel.receive().await? {
            ControlMessage::WorkerReady { rpc_port } => Ok(rpc_port),
            ControlMessage::Error { message, .. } => bail!("{message}"),
            _ => bail!("peer returned an unexpected worker response"),
        }
    }

    pub async fn stop_worker(&mut self) -> Result<()> {
        self.channel.send(&ControlMessage::StopWorker).await?;
        match self.channel.receive().await? {
            ControlMessage::WorkerReady { .. } => Ok(()),
            ControlMessage::Error { message, .. } => bail!("{message}"),
            _ => bail!("peer returned an unexpected worker stop response"),
        }
    }

    pub async fn start_network_benchmark(
        &mut self,
        bind_address: IpAddr,
        port: u16,
    ) -> Result<u16> {
        self.channel
            .send(&ControlMessage::StartNetworkBenchmark {
                bind_address: bind_address.to_string(),
                port,
            })
            .await?;
        match self.channel.receive().await? {
            ControlMessage::NetworkBenchmarkReady { port } => Ok(port),
            ControlMessage::Error { message, .. } => bail!("{message}"),
            _ => bail!("peer returned an unexpected benchmark response"),
        }
    }

    pub async fn stop_network_benchmark(&mut self) -> Result<()> {
        self.channel
            .send(&ControlMessage::StopNetworkBenchmark)
            .await?;
        match self.channel.receive().await? {
            ControlMessage::NetworkBenchmarkReady { .. } => Ok(()),
            ControlMessage::Error { message, .. } => bail!("{message}"),
            _ => bail!("peer returned an unexpected benchmark stop response"),
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    remote_address: SocketAddr,
    context: ServerContext,
) -> Result<()> {
    let mut channel = NoiseChannel::accept(stream, &context.identity).await?;
    let request = match channel.receive().await? {
        ControlMessage::PairRequest(request) => request,
        _ => bail!("first control message was not a pairing request"),
    };
    validate_distinct_identity(context.settings.lock().await.node_id, request.node_id)?;
    if request.protocol_version != PROTOCOL_VERSION
        || request.public_key != channel.remote_public_key()
    {
        reject_pairing(&mut channel, &context, request.node_id).await?;
        bail!("peer identity or protocol version did not match");
    }

    let (trusted, key_mismatch) = {
        let settings = context.settings.lock().await;
        match settings.trusted_peer(request.node_id) {
            Some(peer) => (
                peer.public_key == request.public_key,
                peer.public_key != request.public_key,
            ),
            None => (false, false),
        }
    };
    if key_mismatch {
        reject_pairing(&mut channel, &context, request.node_id).await?;
        bail!("a trusted node presented a different public key");
    }
    let decision = if trusted {
        PairDecision::Accept
    } else {
        let local = context.settings.lock().await.node_id;
        let code = pairing_code(
            local,
            &context.identity.public_key(),
            request.node_id,
            &request.public_key,
        );
        let (response, receiver) = oneshot::channel();
        context
            .events
            .send(ControlEvent::PairingRequested {
                request: request.clone(),
                remote_address,
                code,
                response,
            })
            .await
            .context("pairing prompt could not be displayed")?;
        tokio::time::timeout(Duration::from_secs(120), receiver)
            .await
            .context("pairing confirmation timed out")?
            .context("pairing confirmation was cancelled")?
    };

    if decision == PairDecision::Accept && !trusted {
        let mut settings = context.settings.lock().await;
        settings.trust(TrustedPeer {
            id: request.node_id,
            name: request.node_name.clone(),
            public_key: request.public_key.clone(),
            paired_at: Utc::now(),
        });
        settings.save(&context.paths)?;
    }
    channel
        .send(&ControlMessage::PairResponse(PairResponse {
            node_id: context.settings.lock().await.node_id,
            public_key: context.identity.public_key(),
            decision,
            signature: String::new(),
        }))
        .await?;
    if decision == PairDecision::Reject {
        return Ok(());
    }

    while let Ok(message) = channel.receive().await {
        match message {
            ControlMessage::Ping { nonce } => {
                channel.send(&ControlMessage::Pong { nonce }).await?;
            }
            ControlMessage::GetNodeInfo => {
                channel
                    .send(&ControlMessage::NodeInfo(node_summary(&context).await))
                    .await?;
            }
            ControlMessage::StartWorker {
                bind_address,
                rpc_port,
            } => {
                let address: IpAddr = bind_address.parse().context("invalid RPC bind address")?;
                if !allowed_rpc_bind(address, &interfaces()?) {
                    channel
                        .send(&ControlMessage::Error {
                            message: "RPC worker was not opened on an untrusted interface".into(),
                            detail: Some(format!("Rejected bind address {address}")),
                        })
                        .await?;
                    continue;
                }
                let Some(binaries) = &context.binaries else {
                    channel
                        .send(&ControlMessage::Error {
                            message: "llama.cpp binaries are not configured on this worker".into(),
                            detail: None,
                        })
                        .await?;
                    continue;
                };
                let process = binaries.worker_process(&WorkerConfig {
                    bind_address: address,
                    port: rpc_port,
                    threads: context.hardware.logical_cores.saturating_div(2).max(1),
                    enable_cache: true,
                })?;
                let mut manager = context.processes.lock().await;
                if manager.is_running(ProcessKind::RpcWorker) {
                    manager.stop(ProcessKind::RpcWorker).await?;
                }
                manager.start(process).await?;
                channel
                    .send(&ControlMessage::WorkerReady { rpc_port })
                    .await?;
                let _ = context
                    .events
                    .send(ControlEvent::WorkerStarted { remote_address })
                    .await;
            }
            ControlMessage::StopWorker => {
                context
                    .processes
                    .lock()
                    .await
                    .stop(ProcessKind::RpcWorker)
                    .await?;
                channel
                    .send(&ControlMessage::WorkerReady { rpc_port: 0 })
                    .await?;
                let _ = context
                    .events
                    .send(ControlEvent::WorkerStopped { remote_address })
                    .await;
            }
            ControlMessage::StartNetworkBenchmark { bind_address, port } => {
                let address: IpAddr = bind_address
                    .parse()
                    .context("invalid benchmark bind address")?;
                if !allowed_rpc_bind(address, &interfaces()?) {
                    channel
                        .send(&ControlMessage::Error {
                            message: "Network test was not opened on an untrusted interface".into(),
                            detail: Some(format!("Rejected bind address {address}")),
                        })
                        .await?;
                    continue;
                }
                let mut benchmark = context.network_benchmark.lock().await;
                if let Some(server) = benchmark.take() {
                    server.stop().await;
                }
                let server = BenchmarkServer::start(SocketAddr::new(address, port)).await?;
                let actual_port = server.local_addr().port();
                *benchmark = Some(server);
                channel
                    .send(&ControlMessage::NetworkBenchmarkReady { port: actual_port })
                    .await?;
            }
            ControlMessage::StopNetworkBenchmark => {
                if let Some(server) = context.network_benchmark.lock().await.take() {
                    server.stop().await;
                }
                channel
                    .send(&ControlMessage::NetworkBenchmarkReady { port: 0 })
                    .await?;
            }
            _ => {
                channel
                    .send(&ControlMessage::Error {
                        message: "This control request is not supported".into(),
                        detail: None,
                    })
                    .await?;
            }
        }
    }
    Ok(())
}

async fn reject_pairing(
    channel: &mut NoiseChannel,
    context: &ServerContext,
    remote_id: uuid::Uuid,
) -> Result<()> {
    channel
        .send(&ControlMessage::PairResponse(PairResponse {
            node_id: context.settings.lock().await.node_id,
            public_key: context.identity.public_key(),
            decision: PairDecision::Reject,
            signature: format!("rejected:{remote_id}"),
        }))
        .await
}

async fn node_summary(context: &ServerContext) -> NodeSummary {
    let settings = context.settings.lock().await;
    NodeSummary {
        id: settings.node_id,
        name: settings.node_name.clone(),
        role: settings.role,
        status: NodeStatus::Ready,
        addresses: interfaces()
            .unwrap_or_default()
            .into_iter()
            .filter(|interface| interface.usable_for_cluster())
            .map(|interface| interface.address)
            .collect(),
        control_port: settings.control_port,
        rpc_port: settings.rpc_port,
        available_memory_bytes: context.hardware.ai_memory_bytes,
        compute_backend: context.hardware.backend.to_string(),
    }
}

fn allowed_rpc_bind(address: IpAddr, available: &[NetworkInterface]) -> bool {
    available
        .iter()
        .any(|interface| interface.address == address && interface.usable_for_cluster())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::InterfaceKind;
    use uuid::Uuid;

    #[test]
    fn refuses_vpn_and_unknown_rpc_bind_addresses() {
        let interfaces = [NetworkInterface {
            name: "Ethernet".into(),
            kind: InterfaceKind::Ethernet,
            address: "192.168.1.18".parse().unwrap(),
            prefix_len: 24,
            is_vpn: false,
        }];
        assert!(allowed_rpc_bind(
            "192.168.1.18".parse().unwrap(),
            &interfaces
        ));
        assert!(!allowed_rpc_bind("0.0.0.0".parse().unwrap(), &interfaces));
        assert!(!allowed_rpc_bind(
            "192.168.1.19".parse().unwrap(),
            &interfaces
        ));
    }

    #[tokio::test]
    async fn pairs_over_noise_and_persists_trust() {
        let directory = std::env::temp_dir().join(format!("bution-control-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let paths = AppPaths {
            data_dir: directory.clone(),
            models_dir: directory.join("models"),
            settings_file: directory.join("settings.toml"),
            identity_file: directory.join("identity.key"),
            noise_identity_file: directory.join("noise-identity.key"),
            cache_dir: directory.join("cache"),
        };
        let server_settings = Arc::new(Mutex::new(Settings {
            node_name: "Server Node".into(),
            ..Settings::default()
        }));
        let server_identity = NoiseIdentity::load_or_create(&paths.noise_identity_file).unwrap();
        let mut server = ControlServer::start(
            "127.0.0.1:0".parse().unwrap(),
            server_settings.clone(),
            paths.clone(),
            server_identity,
            None,
            HardwareProfile::detect(),
        )
        .await
        .unwrap();
        let client_path = directory.join("client-noise.key");
        let client_identity = NoiseIdentity::load_or_create(&client_path).unwrap();
        let client_settings = Settings {
            node_name: "Client Node".into(),
            ..Settings::default()
        };
        let address = server.local_addr();
        let client_task = tokio::spawn(async move {
            ControlClient::pair(address, &client_identity, &client_settings).await
        });
        let event = server.next_event().await.unwrap();
        match event {
            ControlEvent::PairingRequested {
                request, response, ..
            } => {
                assert_eq!(request.node_name, "Client Node");
                response.send(PairDecision::Accept).unwrap();
            }
            _ => panic!("expected pairing request"),
        }
        let mut client = client_task.await.unwrap().unwrap();
        let info = client.node_info().await.unwrap();
        assert_eq!(info.name, "Server Node");
        assert_eq!(server_settings.lock().await.trusted_peers.len(), 1);
        drop(client);
        server.stop().await;
        std::fs::remove_dir_all(directory).unwrap();
    }
}
