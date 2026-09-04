use crate::chat::{ChatEvent, ChatMessage, ChatRole};
use crate::cluster::{NodeRole, NodeStatus, NodeSummary};
use crate::hardware::HardwareProfile;
use crate::models::ModelInfo;
use crate::network::{MeasuredRoute, NetworkInterface, interfaces};
use crate::runtime::{RuntimeCommand, RuntimeEvent};
use crate::storage::{AppPaths, Settings};
use crate::telemetry::{TelemetryCollector, TelemetrySample};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::VecDeque;
use tokio::sync::oneshot;

pub struct PendingPairing {
    pub name: String,
    pub address: std::net::SocketAddr,
    pub code: String,
    pub accept_selected: bool,
    response: Option<oneshot::Sender<crate::cluster::PairDecision>>,
}

pub enum AppAction {
    None,
    Runtime(RuntimeCommand),
    SendChat(Vec<ChatMessage>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Cluster,
    Nodes,
    Models,
    Benchmark,
    Chat,
    Settings,
}

impl Screen {
    pub const ALL: [Self; 6] = [
        Self::Cluster,
        Self::Nodes,
        Self::Models,
        Self::Benchmark,
        Self::Chat,
        Self::Settings,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cluster => "Cluster",
            Self::Nodes => "Nodes",
            Self::Models => "Models",
            Self::Benchmark => "Benchmark",
            Self::Chat => "Chat",
            Self::Settings => "Settings",
        }
    }
}

pub struct App {
    pub running: bool,
    pub screen_index: usize,
    pub settings: Settings,
    pub paths: AppPaths,
    pub hardware: HardwareProfile,
    pub interfaces: Vec<NetworkInterface>,
    pub nodes: Vec<NodeSummary>,
    pub model: Option<ModelInfo>,
    pub telemetry: TelemetrySample,
    pub logs: VecDeque<String>,
    pub pending_pairing: Option<PendingPairing>,
    pub cluster_running: bool,
    pub distribution: Vec<(String, f64)>,
    pub best_route: Option<MeasuredRoute>,
    pub chat_messages: Vec<ChatMessage>,
    pub chat_input: String,
    pub chat_streaming: bool,
    telemetry_collector: TelemetryCollector,
}

impl App {
    pub fn load() -> Result<Self> {
        let paths = AppPaths::discover()?;
        let settings = Settings::load_or_create(&paths)?;
        let hardware = HardwareProfile::detect();
        let interfaces = interfaces().unwrap_or_default();
        let model = settings
            .last_model
            .as_ref()
            .and_then(|path| ModelInfo::inspect(path).ok());
        let nodes = vec![NodeSummary {
            id: settings.node_id,
            name: settings.node_name.clone(),
            role: settings.role,
            status: NodeStatus::Ready,
            addresses: interfaces
                .iter()
                .filter(|interface| interface.usable_for_cluster())
                .map(|interface| interface.address)
                .collect(),
            control_port: settings.control_port,
            rpc_port: settings.rpc_port,
            available_memory_bytes: hardware.ai_memory_bytes,
            compute_backend: hardware.backend.to_string(),
        }];
        let mut telemetry_collector = TelemetryCollector::default();
        let telemetry = telemetry_collector.sample();
        let mut logs = VecDeque::new();
        logs.push_back("BUTION node initialized".into());
        logs.push_back("Searching for trusted nodes on the LAN…".into());
        Ok(Self {
            running: true,
            screen_index: 0,
            settings,
            paths,
            hardware,
            interfaces,
            nodes,
            model,
            telemetry,
            logs,
            pending_pairing: None,
            cluster_running: false,
            distribution: Vec::new(),
            best_route: None,
            chat_messages: Vec::new(),
            chat_input: String::new(),
            chat_streaming: false,
            telemetry_collector,
        })
    }

    pub fn screen(&self) -> Screen {
        Screen::ALL[self.screen_index]
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if let Some(pairing) = &mut self.pending_pairing {
            match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                    pairing.accept_selected = !pairing.accept_selected;
                }
                KeyCode::Enter => {
                    let decision = if pairing.accept_selected {
                        crate::cluster::PairDecision::Accept
                    } else {
                        crate::cluster::PairDecision::Reject
                    };
                    if let Some(response) = pairing.response.take() {
                        let _ = response.send(decision);
                    }
                    self.push_log(format!("Pairing request {decision:?}"));
                    self.pending_pairing = None;
                }
                KeyCode::Esc | KeyCode::Char('q' | 'Q') => {
                    if let Some(response) = pairing.response.take() {
                        let _ = response.send(crate::cluster::PairDecision::Reject);
                    }
                    self.pending_pairing = None;
                }
                _ => {}
            }
            return AppAction::None;
        }
        if self.screen() == Screen::Chat {
            match (key.modifiers, key.code) {
                (KeyModifiers::CONTROL, KeyCode::Char('n' | 'N' | 'l' | 'L')) => {
                    self.chat_messages.clear();
                    self.chat_input.clear();
                    self.chat_streaming = false;
                    self.telemetry_collector.set_generation_speed(None);
                    return AppAction::None;
                }
                (_, KeyCode::Esc) => {
                    self.screen_index = 0;
                    return AppAction::None;
                }
                (_, KeyCode::Backspace) if !self.chat_streaming => {
                    self.chat_input.pop();
                    return AppAction::None;
                }
                (_, KeyCode::Enter)
                    if !self.chat_streaming && !self.chat_input.trim().is_empty() =>
                {
                    let content = self.chat_input.trim().to_owned();
                    self.chat_input.clear();
                    self.chat_messages.push(ChatMessage {
                        role: ChatRole::User,
                        content,
                    });
                    self.chat_messages.push(ChatMessage {
                        role: ChatRole::Assistant,
                        content: String::new(),
                    });
                    self.chat_streaming = true;
                    self.telemetry_collector.set_generation_speed(None);
                    let request = self.chat_messages[..self.chat_messages.len() - 1].to_vec();
                    return AppAction::SendChat(request);
                }
                (modifiers, KeyCode::Char(character))
                    if !modifiers.contains(KeyModifiers::CONTROL) && !self.chat_streaming =>
                {
                    self.chat_input.push(character);
                    return AppAction::None;
                }
                _ => return AppAction::None,
            }
        }
        match key.code {
            KeyCode::Char('q' | 'Q') => self.running = false,
            KeyCode::Up | KeyCode::Left => {
                self.screen_index = self.screen_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Right => {
                self.screen_index = (self.screen_index + 1).min(Screen::ALL.len() - 1);
            }
            KeyCode::Esc => self.screen_index = 0,
            KeyCode::Enter if self.screen() == Screen::Cluster => {
                if self.cluster_running {
                    return AppAction::Runtime(RuntimeCommand::StopModel);
                }
                if let Some(model) = &self.model {
                    let name = model.name.clone();
                    let path = model.path.clone();
                    self.push_log(format!("Starting {name}…"));
                    return AppAction::Runtime(RuntimeCommand::StartModel(path));
                }
                self.push_log("Select a GGUF model with --model first".into());
            }
            KeyCode::Char(' ' | 'r' | 'R')
                if self.screen() == Screen::Cluster || self.screen() == Screen::Settings =>
            {
                self.settings.role = match self.settings.role {
                    NodeRole::Automatic => NodeRole::Main,
                    NodeRole::Main => NodeRole::Worker,
                    NodeRole::Worker => NodeRole::Automatic,
                };
                if let Some(local) = self.nodes.first_mut() {
                    local.role = self.settings.role;
                }
                if let Err(error) = self.settings.save(&self.paths) {
                    self.push_log(format!("Settings could not be saved: {error}"));
                } else {
                    self.push_log(format!("Node role changed to {:?}", self.settings.role));
                }
            }
            _ => {}
        }
        AppAction::None
    }

    pub fn refresh_telemetry(&mut self) {
        self.telemetry = self.telemetry_collector.sample();
    }

    pub fn push_log(&mut self, message: String) {
        self.logs.push_back(message);
        while self.logs.len() > 100 {
            self.logs.pop_front();
        }
    }

    pub fn apply_runtime_event(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::NodeDiscovered(node) => {
                if !self.nodes.iter().any(|existing| existing.id == node.id) {
                    self.nodes.push(NodeSummary {
                        id: node.id,
                        name: node.name.clone(),
                        role: NodeRole::Automatic,
                        status: NodeStatus::Discovered,
                        addresses: node.addresses,
                        control_port: node.control_port,
                        rpc_port: 50_052,
                        available_memory_bytes: 0,
                        compute_backend: node.backend,
                    });
                    self.push_log(format!("Discovered {}", node.name));
                }
            }
            RuntimeEvent::NodePaired(node) => {
                self.nodes.retain(|existing| existing.id != node.id);
                self.push_log(format!("Paired with {}", node.name));
                self.nodes.push(node);
            }
            RuntimeEvent::NetworkMeasured { node_id: _, route } => {
                self.push_log(format!(
                    "Selected {}: {:.0} Mbps, {:.1} ms",
                    route.interface.kind,
                    route.benchmark.bandwidth.megabits_per_second,
                    route.benchmark.latency.average_ms
                ));
                self.best_route = Some(route);
            }
            RuntimeEvent::PairingRequested {
                name,
                address,
                code,
                response,
            } => {
                self.pending_pairing = Some(PendingPairing {
                    name,
                    address,
                    code,
                    accept_selected: true,
                    response: Some(response),
                });
            }
            RuntimeEvent::ClusterStarted { distribution } => {
                self.cluster_running = true;
                self.distribution = distribution;
                self.push_log("llama-server started".into());
            }
            RuntimeEvent::ClusterStopped => {
                self.cluster_running = false;
                self.distribution.clear();
                self.push_log("Cluster stopped cleanly".into());
            }
            RuntimeEvent::Log(message) => self.push_log(message),
            RuntimeEvent::Error { message, .. } => self.push_log(message),
        }
    }

    pub fn apply_chat_event(&mut self, event: ChatEvent) {
        match event {
            ChatEvent::Token(token) => {
                if let Some(message) = self.chat_messages.last_mut() {
                    if message.role == ChatRole::Assistant {
                        message.content.push_str(&token);
                    }
                }
            }
            ChatEvent::Finished { tokens_per_second } => {
                self.chat_streaming = false;
                self.telemetry_collector
                    .set_generation_speed(tokens_per_second);
                self.refresh_telemetry();
            }
            ChatEvent::Error { message, .. } => {
                self.chat_streaming = false;
                if let Some(last) = self.chat_messages.last_mut() {
                    if last.role == ChatRole::Assistant && last.content.is_empty() {
                        last.content = format!("{message}. Press Enter to try again.");
                    }
                }
                self.push_log(message);
            }
        }
    }
}
