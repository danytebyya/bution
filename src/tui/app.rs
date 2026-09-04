use crate::cluster::{NodeRole, NodeStatus, NodeSummary};
use crate::hardware::HardwareProfile;
use crate::models::ModelInfo;
use crate::network::{NetworkInterface, interfaces};
use crate::storage::{AppPaths, Settings};
use crate::telemetry::{TelemetryCollector, TelemetrySample};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use std::collections::VecDeque;

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
            telemetry_collector,
        })
    }

    pub fn screen(&self) -> Screen {
        Screen::ALL[self.screen_index]
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q' | 'Q') => self.running = false,
            KeyCode::Up | KeyCode::Left => {
                self.screen_index = self.screen_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Right => {
                self.screen_index = (self.screen_index + 1).min(Screen::ALL.len() - 1);
            }
            KeyCode::Esc => self.screen_index = 0,
            KeyCode::Char(' ') if self.screen() == Screen::Settings => {
                self.settings.role = match self.settings.role {
                    NodeRole::Automatic => NodeRole::Main,
                    NodeRole::Main => NodeRole::Worker,
                    NodeRole::Worker => NodeRole::Automatic,
                };
                if let Err(error) = self.settings.save(&self.paths) {
                    self.push_log(format!("Settings could not be saved: {error}"));
                } else {
                    self.push_log(format!("Node role changed to {:?}", self.settings.role));
                }
            }
            _ => {}
        }
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
}
