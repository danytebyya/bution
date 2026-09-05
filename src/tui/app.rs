use crate::chat::{ChatEvent, ChatMessage, ChatRole};
use crate::cluster::{NodeRole, NodeStatus, NodeSummary};
use crate::hardware::HardwareProfile;
use crate::locale::Language;
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

    pub fn label(self, language: Language) -> &'static str {
        match self {
            Self::Cluster => language.text("Cluster", "Кластер"),
            Self::Nodes => language.text("Nodes", "Узлы"),
            Self::Models => language.text("Model", "Модель"),
            Self::Benchmark => language.text("Network", "Сеть"),
            Self::Chat => language.text("Chat", "Чат"),
            Self::Settings => language.text("Settings", "Настройки"),
        }
    }
}

pub struct App {
    pub language: Language,
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
    pub last_error: Option<String>,
    pub pending_pairing: Option<PendingPairing>,
    pub cluster_running: bool,
    pub distribution: Vec<(String, f64)>,
    pub best_route: Option<MeasuredRoute>,
    pub chat_messages: Vec<ChatMessage>,
    pub chat_input: String,
    pub chat_streaming: bool,
    pub update_available: Option<String>,
    telemetry_collector: TelemetryCollector,
}

impl App {
    pub fn load() -> Result<Self> {
        let language = Language::detect();
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
        logs.push_back(
            language
                .text("BUTION is ready", "BUTION готов к работе")
                .into(),
        );
        logs.push_back(
            language
                .text(
                    "Searching for nodes on the LAN…",
                    "Поиск узлов в локальной сети…",
                )
                .into(),
        );
        Ok(Self {
            language,
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
            last_error: None,
            pending_pairing: None,
            cluster_running: false,
            distribution: Vec::new(),
            best_route: None,
            chat_messages: Vec::new(),
            chat_input: String::new(),
            chat_streaming: false,
            update_available: None,
            telemetry_collector,
        })
    }

    pub fn screen(&self) -> Screen {
        Screen::ALL[self.screen_index]
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppAction {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('q' | 'Q'))
        {
            self.running = false;
            return AppAction::None;
        }
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
                    self.push_log(
                        self.language
                            .text(
                                if decision == crate::cluster::PairDecision::Accept {
                                    "Pairing accepted"
                                } else {
                                    "Pairing rejected"
                                },
                                if decision == crate::cluster::PairDecision::Accept {
                                    "Подключение разрешено"
                                } else {
                                    "Подключение отклонено"
                                },
                            )
                            .into(),
                    );
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
                (KeyModifiers::CONTROL, KeyCode::Char('q' | 'Q' | 'c' | 'C')) => {
                    self.running = false;
                    return AppAction::None;
                }
                (KeyModifiers::CONTROL, KeyCode::Char('n' | 'N' | 'l' | 'L'))
                    if !self.chat_streaming =>
                {
                    self.chat_messages.clear();
                    self.chat_input.clear();
                    self.chat_streaming = false;
                    self.telemetry_collector.set_generation_speed(None);
                    return AppAction::None;
                }
                (_, KeyCode::Tab) => {
                    self.screen_index = (self.screen_index + 1) % Screen::ALL.len();
                    return AppAction::None;
                }
                (_, KeyCode::BackTab) => {
                    self.screen_index = if self.screen_index == 0 {
                        Screen::ALL.len() - 1
                    } else {
                        self.screen_index - 1
                    };
                    return AppAction::None;
                }
                (_, KeyCode::Esc) => {
                    if !self.chat_input.is_empty() {
                        self.chat_input.clear();
                    } else {
                        self.screen_index = self.screen_index.saturating_sub(1);
                    }
                    return AppAction::None;
                }
                (modifiers, KeyCode::Left | KeyCode::Up)
                    if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.screen_index = self.screen_index.saturating_sub(1);
                    return AppAction::None;
                }
                (modifiers, KeyCode::Right | KeyCode::Down)
                    if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.screen_index = (self.screen_index + 1).min(Screen::ALL.len() - 1);
                    return AppAction::None;
                }
                (_, KeyCode::Left | KeyCode::Up) if self.chat_input.is_empty() => {
                    self.screen_index = self.screen_index.saturating_sub(1);
                    return AppAction::None;
                }
                (_, KeyCode::Right | KeyCode::Down) if self.chat_input.is_empty() => {
                    self.screen_index = (self.screen_index + 1).min(Screen::ALL.len() - 1);
                    return AppAction::None;
                }
                (_, KeyCode::Backspace) if !self.chat_streaming => {
                    self.chat_input.pop();
                    return AppAction::None;
                }
                (_, KeyCode::Enter)
                    if self.cluster_running
                        && !self.chat_streaming
                        && !self.chat_input.trim().is_empty() =>
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
            KeyCode::Tab => {
                self.screen_index = (self.screen_index + 1) % Screen::ALL.len();
            }
            KeyCode::BackTab => {
                self.screen_index = if self.screen_index == 0 {
                    Screen::ALL.len() - 1
                } else {
                    self.screen_index - 1
                };
            }
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
                if self.settings.role == NodeRole::Worker {
                    return AppAction::None;
                }
                if let Some(model) = &self.model {
                    self.last_error = None;
                    let name = model.name.clone();
                    let path = model.path.clone();
                    self.push_log(format!(
                        "{}: {name}",
                        self.language.text("Starting model", "Запуск модели")
                    ));
                    return AppAction::Runtime(RuntimeCommand::StartModel(path));
                }
                self.push_log(
                    self.language
                        .text(
                            "Select a GGUF model with --model first",
                            "Сначала выберите модель через --model",
                        )
                        .into(),
                );
            }
            KeyCode::Char(' ' | 'r' | 'R')
                if self.screen() == Screen::Cluster && !self.cluster_running =>
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
                    self.push_log(format!(
                        "{}: {error}",
                        self.language
                            .text("Could not save settings", "Не удалось сохранить настройки")
                    ));
                } else {
                    self.push_log(format!(
                        "{}: {}",
                        self.language.text("Role", "Роль"),
                        self.language.role(self.settings.role)
                    ));
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
                    self.push_log(format!(
                        "{}: {}",
                        self.language.text("Node discovered", "Обнаружен узел"),
                        node.name
                    ));
                }
            }
            RuntimeEvent::NodePaired(node) => {
                self.nodes.retain(|existing| existing.id != node.id);
                self.push_log(format!(
                    "{}: {}",
                    self.language.text("Connected", "Подключён"),
                    node.name
                ));
                self.nodes.push(node);
            }
            RuntimeEvent::NetworkMeasured { node_id: _, route } => {
                self.push_log(format!(
                    "{} {}: {:.0} Mbps, {:.1} ms",
                    self.language.text("Selected", "Выбран"),
                    route.interface.name,
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
                self.last_error = None;
                self.cluster_running = true;
                self.distribution = distribution;
                self.push_log(
                    self.language
                        .text("Model process started", "Процесс модели запущен")
                        .into(),
                );
            }
            RuntimeEvent::ClusterStopped => {
                self.last_error = None;
                self.cluster_running = false;
                self.distribution.clear();
                self.push_log(
                    self.language
                        .text("Model stopped", "Модель остановлена")
                        .into(),
                );
            }
            RuntimeEvent::UpdateAvailable { latest_version, .. } => {
                self.update_available = Some(latest_version);
            }
            RuntimeEvent::Log(message) => self.push_log(message),
            RuntimeEvent::Error { message, .. } => {
                self.last_error = Some(message.clone());
                self.push_log(message);
            }
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
                        last.content = message.clone();
                    }
                }
                self.push_log(message);
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn make_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    pub(in crate::tui) fn test_app() -> App {
        let mut collector = TelemetryCollector::default();
        let telemetry = collector.sample();
        App {
            language: Language::English,
            running: true,
            screen_index: 0,
            settings: Settings::default(),
            paths: AppPaths::discover().unwrap(),
            hardware: HardwareProfile::detect(),
            interfaces: Vec::new(),
            nodes: Vec::new(),
            model: None,
            telemetry,
            logs: VecDeque::new(),
            last_error: None,
            pending_pairing: None,
            cluster_running: false,
            distribution: Vec::new(),
            best_route: None,
            chat_messages: Vec::new(),
            chat_input: String::new(),
            chat_streaming: false,
            update_available: None,
            telemetry_collector: collector,
        }
    }

    #[test]
    fn arrow_keys_navigate_across_all_screens_including_chat() {
        let mut app = test_app();
        assert_eq!(app.screen(), Screen::Cluster);

        // Move forward: 0 -> 1 -> 2 -> 3 -> 4 (Chat) -> 5 (Settings)
        app.handle_key(make_key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Nodes);

        app.handle_key(make_key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Models);

        app.handle_key(make_key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Benchmark);

        app.handle_key(make_key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Chat);

        // Navigating right from empty Chat screen must reach Settings!
        app.handle_key(make_key(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Settings);

        // Navigating left from Settings must reach Chat
        app.handle_key(make_key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Chat);

        // Navigating left from empty Chat must reach Benchmark
        app.handle_key(make_key(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Benchmark);
    }

    #[test]
    fn tab_and_backtab_cycle_screens_even_when_chat_has_input() {
        let mut app = test_app();
        app.screen_index = 4; // Chat screen
        app.chat_input = "Hello LLM".into();

        // Tab moves to Settings (5)
        app.handle_key(make_key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Settings);

        // Tab wraps around to Cluster (0)
        app.handle_key(make_key(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Cluster);

        // BackTab wraps to Settings (5)
        app.handle_key(make_key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.screen(), Screen::Settings);

        // BackTab moves to Chat (4)
        app.handle_key(make_key(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(app.screen(), Screen::Chat);
    }

    #[test]
    fn chat_esc_clears_input_then_navigates_back() {
        let mut app = test_app();
        app.screen_index = 4; // Chat screen
        app.chat_input = "draft".into();

        // First Esc clears input text
        app.handle_key(make_key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.chat_input.is_empty());
        assert_eq!(app.screen(), Screen::Chat);

        // Second Esc navigates back to previous screen (Benchmark)
        app.handle_key(make_key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.screen(), Screen::Benchmark);
    }

    #[test]
    fn role_changes_only_on_idle_cluster_page() {
        let mut app = test_app();
        for index in [1, 2, 3, 5] {
            app.screen_index = index;
            app.handle_key(make_key(KeyCode::Char(' '), KeyModifiers::NONE));
            assert_eq!(app.settings.role, NodeRole::Automatic);
        }
        app.screen_index = 0;
        app.cluster_running = true;
        app.handle_key(make_key(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.settings.role, NodeRole::Automatic);

        let temporary =
            std::env::temp_dir().join(format!("bution-role-test-{}", uuid::Uuid::new_v4()));
        app.paths.data_dir = temporary.clone();
        app.paths.settings_file = temporary.join("settings.toml");
        app.paths.cache_dir = temporary.join("cache");
        app.cluster_running = false;
        app.handle_key(make_key(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.settings.role, NodeRole::Main);
        assert_eq!(
            Settings::load_or_create(&app.paths).unwrap().role,
            NodeRole::Main
        );
        std::fs::remove_dir_all(temporary).unwrap();
    }

    #[test]
    fn chat_send_requires_started_model() {
        let mut app = test_app();
        app.screen_index = 4;
        app.chat_input = "Hello".into();
        assert!(matches!(
            app.handle_key(make_key(KeyCode::Enter, KeyModifiers::NONE)),
            AppAction::None
        ));
        assert!(!app.chat_streaming);
        app.cluster_running = true;
        assert!(matches!(
            app.handle_key(make_key(KeyCode::Enter, KeyModifiers::NONE)),
            AppAction::SendChat(_)
        ));
        assert!(app.chat_streaming);
    }

    #[test]
    fn clear_chat_does_not_discard_an_active_stream() {
        let mut app = test_app();
        app.screen_index = 4;
        app.chat_streaming = true;
        app.chat_input = "draft".into();
        app.handle_key(make_key(KeyCode::Char('n'), KeyModifiers::CONTROL));
        assert_eq!(app.chat_input, "draft");
        assert!(app.chat_streaming);
    }

    #[test]
    fn model_lifecycle_messages_follow_ui_language() {
        for language in [Language::English, Language::Russian] {
            let mut app = test_app();
            app.language = language;
            app.apply_runtime_event(RuntimeEvent::ClusterStarted {
                distribution: Vec::new(),
            });
            assert_eq!(
                app.logs.back().unwrap(),
                language.text("Model process started", "Процесс модели запущен")
            );
            app.apply_runtime_event(RuntimeEvent::ClusterStopped);
            assert_eq!(
                app.logs.back().unwrap(),
                language.text("Model stopped", "Модель остановлена")
            );
        }
    }
}
