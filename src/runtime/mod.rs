//! Background orchestration connecting discovery, pairing, RPC, and llama-server.

use crate::cluster::NodeSummary;
use crate::control::{ControlClient, ControlEvent, ControlServer};
use crate::discovery::{DiscoveredNode, DiscoveryAdvertisement, DiscoveryEvent, MdnsDiscovery};
use crate::hardware::HardwareProfile;
use crate::llama::{LlamaBinaries, ServerConfig};
use crate::models::{ModelInfo, RunRecommendation};
use crate::network::{
    MeasuredRoute, NetworkInterface, route_candidates, run_network_benchmark, select_best_route,
};
use crate::optimizer::{NodeCapacity, plan_distribution};
use crate::processes::{ProcessKind, ProcessManager};
use crate::security::NoiseIdentity;
use crate::storage::{AppPaths, Settings, TrustedPeer};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;

pub enum RuntimeEvent {
    NodeDiscovered(DiscoveredNode),
    NodePaired(NodeSummary),
    NetworkMeasured {
        node_id: uuid::Uuid,
        route: MeasuredRoute,
    },
    PairingRequested {
        name: String,
        address: SocketAddr,
        code: String,
        response: oneshot::Sender<crate::cluster::PairDecision>,
    },
    ClusterStarted {
        distribution: Vec<(String, f64)>,
    },
    ClusterStopped,
    Log(String),
    Error {
        message: String,
        detail: String,
    },
}

pub enum RuntimeCommand {
    StartModel(PathBuf),
    StopModel,
    Shutdown,
}

pub struct RuntimeHandle {
    pub events: mpsc::Receiver<RuntimeEvent>,
    commands: mpsc::Sender<RuntimeCommand>,
    task: JoinHandle<()>,
}

impl RuntimeHandle {
    pub async fn start(
        settings: Settings,
        paths: AppPaths,
        hardware: HardwareProfile,
        local_interfaces: Vec<NetworkInterface>,
    ) -> Result<Self> {
        let identity = NoiseIdentity::load_or_create(&paths.noise_identity_file)?;
        let binaries = LlamaBinaries::discover(settings.llama_bin_dir.as_deref()).ok();
        let settings = Arc::new(Mutex::new(settings));
        let control_port = settings.lock().await.control_port;
        let control = ControlServer::start(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), control_port),
            settings.clone(),
            paths.clone(),
            identity.clone(),
            binaries.clone(),
            hardware.clone(),
        )
        .await?;
        let advertisement = {
            let settings = settings.lock().await;
            DiscoveryAdvertisement {
                id: settings.node_id,
                name: settings.node_name.clone(),
                public_key: identity.public_key(),
                role: format!("{:?}", settings.role).to_ascii_lowercase(),
                backend: hardware.backend.to_string(),
                control_port,
            }
        };
        let discovery = MdnsDiscovery::start(advertisement)?;
        let (events_sender, events) = mpsc::channel(128);
        let (commands, command_receiver) = mpsc::channel(16);
        let task = tokio::spawn(run_loop(
            settings,
            paths,
            hardware,
            local_interfaces,
            identity,
            binaries,
            control,
            discovery,
            events_sender,
            command_receiver,
        ));
        Ok(Self {
            events,
            commands,
            task,
        })
    }

    pub async fn command(&self, command: RuntimeCommand) -> Result<()> {
        self.commands
            .send(command)
            .await
            .context("BUTION background runtime has stopped")
    }

    pub async fn shutdown(self) {
        let _ = self.commands.send(RuntimeCommand::Shutdown).await;
        let _ = self.task.await;
    }
}

struct PairOutcome {
    node: DiscoveredNode,
    result: Result<ControlClient>,
}

#[allow(clippy::too_many_arguments)]
async fn run_loop(
    settings: Arc<Mutex<Settings>>,
    paths: AppPaths,
    hardware: HardwareProfile,
    local_interfaces: Vec<NetworkInterface>,
    identity: NoiseIdentity,
    binaries: Option<LlamaBinaries>,
    mut control: ControlServer,
    discovery: MdnsDiscovery,
    events: mpsc::Sender<RuntimeEvent>,
    mut commands: mpsc::Receiver<RuntimeCommand>,
) {
    let (pair_sender, mut pair_receiver) = mpsc::channel::<PairOutcome>(16);
    let mut clients = HashMap::new();
    let mut best_routes: HashMap<uuid::Uuid, MeasuredRoute> = HashMap::new();
    let mut main_processes = ProcessManager::default();
    if binaries.is_none() {
        let _ = events
            .send(RuntimeEvent::Log(
                "llama.cpp binaries not found; discovery and pairing remain available".into(),
            ))
            .await;
    }

    loop {
        tokio::select! {
            event = discovery.next() => match event {
                Ok(DiscoveryEvent::Found(node)) => {
                    let _ = events.send(RuntimeEvent::NodeDiscovered(node.clone())).await;
                    let local_id = settings.lock().await.node_id;
                    if local_id < node.id && !clients.contains_key(&node.id) {
                        if let Some(address) = preferred_control_address(&node) {
                            let sender = pair_sender.clone();
                            let identity = identity.clone();
                            let local_settings = settings.lock().await.clone();
                            tokio::spawn(async move {
                                let result = ControlClient::pair(address, &identity, &local_settings).await;
                                let _ = sender.send(PairOutcome { node, result }).await;
                            });
                        }
                    }
                }
                Ok(DiscoveryEvent::Removed { fullname }) => {
                    let _ = events.send(RuntimeEvent::Log(format!("Node left the LAN: {fullname}"))).await;
                }
                Err(error) => {
                    let _ = events.send(RuntimeEvent::Error {
                        message: "Local network discovery stopped".into(),
                        detail: format!("{error:#}"),
                    }).await;
                    break;
                }
            },
            event = control.next_event() => match event {
                Some(ControlEvent::PairingRequested { request, remote_address, code, response }) => {
                    let _ = events.send(RuntimeEvent::PairingRequested {
                        name: request.node_name,
                        address: remote_address,
                        code,
                        response,
                    }).await;
                }
                Some(ControlEvent::WorkerStarted { remote_address }) => {
                    let _ = events.send(RuntimeEvent::Log(format!("RPC worker started for {remote_address}"))).await;
                }
                Some(ControlEvent::WorkerStopped { remote_address }) => {
                    let _ = events.send(RuntimeEvent::Log(format!("RPC worker stopped for {remote_address}"))).await;
                }
                Some(ControlEvent::ConnectionError { message, detail }) => {
                    let _ = events.send(RuntimeEvent::Error { message, detail }).await;
                }
                None => break,
            },
            Some(outcome) = pair_receiver.recv() => match outcome.result {
                Ok(mut client) => match client.node_info().await {
                    Ok(info) => {
                        {
                            let mut stored = settings.lock().await;
                            stored.trust(TrustedPeer {
                                id: info.id,
                                name: info.name.clone(),
                                public_key: client.remote_public_key().to_owned(),
                                paired_at: Utc::now(),
                            });
                            let _ = stored.save(&paths);
                        }
                        clients.insert(info.id, client);
                        let _ = events.send(RuntimeEvent::NodePaired(info)).await;
                        if let Some(client) = clients.get_mut(&outcome.node.id) {
                            match benchmark_routes(client, &outcome.node, &local_interfaces).await {
                                Ok(routes) => {
                                    if let Some(best) = select_best_route(&routes).cloned() {
                                        best_routes.insert(outcome.node.id, best.clone());
                                        let _ = events.send(RuntimeEvent::NetworkMeasured { node_id: outcome.node.id, route: best }).await;
                                    }
                                }
                                Err(error) => {
                                    let _ = events.send(RuntimeEvent::Error { message: format!("Network test with {} could not complete", outcome.node.name), detail: format!("{error:#}") }).await;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        let _ = events.send(RuntimeEvent::Error {
                            message: format!("{} paired but did not report its resources", outcome.node.name),
                            detail: format!("{error:#}"),
                        }).await;
                    }
                },
                Err(error) => {
                    let _ = events.send(RuntimeEvent::Error {
                        message: format!("Could not pair with {}", outcome.node.name),
                        detail: format!("{error:#}"),
                    }).await;
                }
            },
            Some(command) = commands.recv() => match command {
                RuntimeCommand::StartModel(model) => {
                    let result = start_model(
                        model,
                        &hardware,
                        &local_interfaces,
                        binaries.as_ref(),
                        &mut clients,
                        &best_routes,
                        &mut main_processes,
                    ).await;
                    match result {
                        Ok(distribution) => { let _ = events.send(RuntimeEvent::ClusterStarted { distribution }).await; }
                        Err(error) => { let _ = events.send(RuntimeEvent::Error { message: "The model could not be started".into(), detail: format!("{error:#}") }).await; }
                    }
                }
                RuntimeCommand::StopModel => {
                    let _ = main_processes.stop(ProcessKind::LlamaServer).await;
                    for client in clients.values_mut() { let _ = client.stop_worker().await; }
                    let _ = events.send(RuntimeEvent::ClusterStopped).await;
                }
                RuntimeCommand::Shutdown => break,
            },
            else => break,
        }
    }
    main_processes.stop_all().await;
    for client in clients.values_mut() {
        let _ = client.stop_worker().await;
    }
    control.stop().await;
}

async fn start_model(
    model_path: PathBuf,
    hardware: &HardwareProfile,
    local_interfaces: &[NetworkInterface],
    binaries: Option<&LlamaBinaries>,
    clients: &mut HashMap<uuid::Uuid, ControlClient>,
    best_routes: &HashMap<uuid::Uuid, MeasuredRoute>,
    processes: &mut ProcessManager,
) -> Result<Vec<(String, f64)>> {
    let binaries = binaries.context("llama.cpp binaries are not configured")?;
    let model = ModelInfo::inspect(&model_path)?;
    let remote_info = if let Some(client) = clients.values_mut().next() {
        Some(client.node_info().await?)
    } else {
        None
    };
    let fit = model.fit_report(
        hardware.ai_memory_bytes,
        &remote_info
            .iter()
            .map(|node| node.available_memory_bytes)
            .collect::<Vec<_>>(),
    );
    if fit.recommendation == RunRecommendation::InsufficientMemory {
        bail!("not enough cluster memory for {}", model.name);
    }

    if processes.is_running(ProcessKind::LlamaServer) {
        processes.stop(ProcessKind::LlamaServer).await?;
    }
    let mut config = ServerConfig::local(model_path);
    let distribution = if fit.recommendation == RunRecommendation::Cluster {
        let remote = remote_info.context("a paired worker is required for this model")?;
        let remote_address = best_routes
            .get(&remote.id)
            .map(|route| route.remote_address)
            .or_else(|| {
                route_candidates(local_interfaces, &remote.addresses)
                    .into_iter()
                    .next()
                    .map(|route| route.1)
            })
            .context("no direct trusted LAN route to the worker")?;
        let client = clients
            .get_mut(&remote.id)
            .context("paired worker connection was lost")?;
        client.start_worker(remote_address, remote.rpc_port).await?;
        config.rpc_endpoints = vec![SocketAddr::new(remote_address, remote.rpc_port)];
        let nodes = [
            NodeCapacity {
                node_id: uuid::Uuid::nil(),
                name: "Local".into(),
                available_memory_bytes: hardware.ai_memory_bytes,
                compute_score: hardware.logical_cores as f64,
                network_score: 100.0,
                local: true,
            },
            NodeCapacity {
                node_id: remote.id,
                name: remote.name.clone(),
                available_memory_bytes: remote.available_memory_bytes,
                compute_score: 1.0,
                network_score: 50.0,
                local: false,
            },
        ];
        let plan = plan_distribution(model.estimated_memory_bytes, &nodes)?;
        config.tensor_split = plan.tensor_split();
        plan.allocations
            .into_iter()
            .map(|allocation| (allocation.name, allocation.fraction))
            .collect()
    } else {
        vec![("Local".into(), 1.0)]
    };
    processes.start(binaries.server_process(&config)?).await?;
    Ok(distribution)
}

fn preferred_control_address(node: &DiscoveredNode) -> Option<SocketAddr> {
    node.addresses
        .iter()
        .copied()
        .find(|address| address.is_ipv4())
        .or_else(|| node.addresses.first().copied())
        .map(|address| SocketAddr::new(address, node.control_port))
}

async fn benchmark_routes(
    client: &mut ControlClient,
    node: &DiscoveredNode,
    local_interfaces: &[NetworkInterface],
) -> Result<Vec<MeasuredRoute>> {
    let candidates = route_candidates(local_interfaces, &node.addresses);
    let mut measurements = Vec::new();
    for (interface, remote_address) in candidates {
        if remote_address.is_ipv6() {
            continue;
        }
        let requested_port = node.control_port.saturating_add(1);
        let port = client
            .start_network_benchmark(remote_address, requested_port)
            .await?;
        let result = run_network_benchmark(SocketAddr::new(remote_address, port)).await;
        let _ = client.stop_network_benchmark().await;
        if let Ok(benchmark) = result {
            measurements.push(MeasuredRoute {
                interface,
                remote_address,
                benchmark,
            });
        }
    }
    if measurements.is_empty() {
        bail!("no LAN route completed the network benchmark");
    }
    Ok(measurements)
}
