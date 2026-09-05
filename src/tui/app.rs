use crate::chat::{ChatEvent, ChatMessage, ChatRole};
use crate::cluster::{NodeRole, NodeStatus, NodeSummary};
use crate::hardware::HardwareProfile;
use crate::hub::download::DownloadProgress;
use crate::hub::huggingface::HubRepository;
use crate::hub::recommendations::{FitRating, MemoryNode, RankedFile, rate_installed};
use crate::hub::{HubCommand, HubEvent};
use crate::locale::Language;
use crate::models::{ModelInfo, scan_directory};
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
    Hub(HubCommand),
    SendChat(Vec<ChatMessage>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsPane {
    Search,
    Repositories,
    Files,
    Installed,
}

pub struct ModelsState {
    pub pane: ModelsPane,
    pub search_input: String,
    pub active_query: String,
    pub searching: bool,
    pub repositories: Vec<HubRepository>,
    pub repository_index: usize,
    pub open_repository: Option<String>,
    pub files: Vec<RankedFile>,
    pub file_index: usize,
    pub installed: Vec<ModelInfo>,
    pub installed_index: usize,
    pub download: Option<DownloadProgress>,
    pub status: Option<String>,
    pub delete_confirmation: Option<std::path::PathBuf>,
    pub delete_after_stop: Option<std::path::PathBuf>,
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
            Self::Models => language.text("Models", "Модели"),
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
    pub models: ModelsState,
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
        let installed = scan_directory(&paths.models_dir).unwrap_or_default();
        let nodes = vec![NodeSummary {
            id: settings.node_id,
            name: settings.node_name.clone(),
            role: settings.role,
            status: NodeStatus::Ready,
            addresses: crate::network::filter_display_addresses(
                &interfaces
                    .iter()
                    .filter(|interface| interface.usable_for_cluster())
                    .map(|interface| interface.address)
                    .collect::<Vec<_>>(),
            ),
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
            models: ModelsState {
                pane: ModelsPane::Search,
                search_input: String::new(),
                active_query: String::new(),
                searching: false,
                repositories: Vec::new(),
                repository_index: 0,
                open_repository: None,
                files: Vec::new(),
                file_index: 0,
                installed,
                installed_index: 0,
                download: None,
                status: None,
                delete_confirmation: None,
                delete_after_stop: None,
            },
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
        if self.screen() == Screen::Models {
            return self.handle_models_key(key);
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

    fn handle_models_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Char('q' | 'Q') if self.models.pane != ModelsPane::Search => {
                self.running = false;
            }
            KeyCode::Left
                if self.models.pane != ModelsPane::Search
                    || self.models.search_input.is_empty() =>
            {
                self.screen_index = self.screen_index.saturating_sub(1);
            }
            KeyCode::Right
                if self.models.pane != ModelsPane::Search
                    || self.models.search_input.is_empty() =>
            {
                self.screen_index = (self.screen_index + 1).min(Screen::ALL.len() - 1);
            }
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
            KeyCode::Char('i' | 'I') if self.models.pane != ModelsPane::Search => {
                self.models.pane = ModelsPane::Installed;
                self.models.delete_confirmation = None;
            }
            KeyCode::Char('/') if self.models.pane != ModelsPane::Search => {
                self.models.pane = ModelsPane::Search;
                self.models.delete_confirmation = None;
            }
            KeyCode::Char('c' | 'C') if self.models.download.is_some() => {
                self.models.status = Some(
                    self.language
                        .text("Cancelling download…", "Отмена загрузки…")
                        .into(),
                );
                return AppAction::Hub(HubCommand::CancelDownload);
            }
            KeyCode::Esc if self.models.download.is_some() => {
                self.models.status = Some(
                    self.language
                        .text("Cancelling download…", "Отмена загрузки…")
                        .into(),
                );
                return AppAction::Hub(HubCommand::CancelDownload);
            }
            KeyCode::Esc => {
                self.models.delete_confirmation = None;
                self.models.pane = match self.models.pane {
                    ModelsPane::Files | ModelsPane::Installed => ModelsPane::Repositories,
                    ModelsPane::Repositories => ModelsPane::Search,
                    ModelsPane::Search => {
                        if !self.models.search_input.is_empty() {
                            self.models.search_input.clear();
                        } else {
                            self.screen_index = 0;
                        }
                        ModelsPane::Search
                    }
                };
            }
            KeyCode::Up => self.move_model_selection(-1),
            KeyCode::Down => self.move_model_selection(1),
            KeyCode::Enter => match self.models.pane {
                ModelsPane::Search => {
                    let query = self.models.search_input.trim().to_owned();
                    if !query.is_empty() {
                        self.models.searching = true;
                        self.models.status = None;
                        return AppAction::Hub(HubCommand::Search(query));
                    }
                }
                ModelsPane::Repositories => {
                    if let Some(repository) =
                        self.models.repositories.get(self.models.repository_index)
                    {
                        let id = repository.id.clone();
                        self.models.open_repository = Some(id.clone());
                        self.models.status = Some(
                            self.language
                                .text("Loading GGUF files…", "Загрузка списка GGUF…")
                                .into(),
                        );
                        return AppAction::Hub(HubCommand::OpenRepository(
                            id,
                            self.recommendation_nodes(),
                        ));
                    }
                }
                ModelsPane::Files => {
                    if self.models.download.is_none() {
                        if let Some(file) = self.models.files.get(self.models.file_index) {
                            let basename = std::path::Path::new(&file.file.filename)
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or(&file.file.filename)
                                .to_owned();
                            self.models.download = Some(DownloadProgress {
                                destination: self.paths.models_dir.join(&basename),
                                filename: basename,
                                downloaded_bytes: 0,
                                total_bytes: file.file.size_bytes,
                                bytes_per_second: 0.0,
                            });
                            self.models.status = Some(
                                self.language
                                    .text("Starting download…", "Начало загрузки…")
                                    .into(),
                            );
                            return AppAction::Hub(HubCommand::Download(file.file.clone()));
                        }
                    }
                }
                ModelsPane::Installed => {
                    if let Some(model) = self
                        .models
                        .installed
                        .get(self.models.installed_index)
                        .cloned()
                    {
                        if self.cluster_running {
                            self.models.status = Some(
                                self.language
                                    .text(
                                        "Stop inference before changing the active model",
                                        "Остановите inference перед сменой активной модели",
                                    )
                                    .into(),
                            );
                        } else {
                            self.model = Some(model.clone());
                            self.settings.last_model = Some(model.path.clone());
                            match self.settings.save(&self.paths) {
                                Ok(()) => {
                                    self.models.status = Some(
                                        self.language
                                            .text("Model selected", "Модель выбрана")
                                            .into(),
                                    )
                                }
                                Err(error) => self.models.status = Some(format!("{error:#}")),
                            }
                        }
                    }
                }
            },
            KeyCode::Char('d' | 'D') if self.models.pane == ModelsPane::Installed => {
                if let Some(model) = self.models.installed.get(self.models.installed_index) {
                    let path = model.path.clone();
                    if self.models.delete_confirmation.as_ref() != Some(&path) {
                        self.models.delete_confirmation = Some(path);
                        self.models.status = Some(
                            self.language
                                .text(
                                    "Press D again to confirm deletion",
                                    "Нажмите D ещё раз для подтверждения удаления",
                                )
                                .into(),
                        );
                    } else if self.cluster_running
                        && self
                            .model
                            .as_ref()
                            .is_some_and(|active| active.path == path)
                    {
                        self.models.delete_after_stop = Some(path);
                        self.models.status = Some(
                            self.language
                                .text(
                                    "Stopping inference before deletion…",
                                    "Остановка inference перед удалением…",
                                )
                                .into(),
                        );
                        return AppAction::Runtime(RuntimeCommand::StopModel);
                    } else {
                        self.models.delete_confirmation = None;
                        return AppAction::Hub(HubCommand::Delete(path));
                    }
                }
            }
            KeyCode::Backspace if self.models.pane == ModelsPane::Search => {
                self.models.search_input.pop();
            }
            KeyCode::Backspace if self.models.pane == ModelsPane::Repositories => {
                self.models.pane = ModelsPane::Search;
                self.models.search_input.pop();
            }
            KeyCode::Char(character)
                if self.models.pane == ModelsPane::Search
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.models.search_input.push(character);
            }
            KeyCode::Char(character)
                if self.models.pane == ModelsPane::Repositories
                    && self.models.repositories.is_empty()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.models.pane = ModelsPane::Search;
                self.models.search_input.push(character);
            }
            _ => {}
        }
        AppAction::None
    }

    fn move_model_selection(&mut self, delta: isize) {
        let (index, length) = match self.models.pane {
            ModelsPane::Repositories => (
                &mut self.models.repository_index,
                self.models.repositories.len(),
            ),
            ModelsPane::Files => (&mut self.models.file_index, self.models.files.len()),
            ModelsPane::Installed => (
                &mut self.models.installed_index,
                self.models.installed.len(),
            ),
            ModelsPane::Search => return,
        };
        if length > 0 {
            *index = (*index as isize + delta).clamp(0, length.saturating_sub(1) as isize) as usize;
        }
        self.models.delete_confirmation = None;
    }

    fn recommendation_nodes(&self) -> Vec<MemoryNode> {
        let mut nodes = vec![MemoryNode {
            name: "Local".into(),
            safe_memory_bytes: self.hardware.ai_memory_bytes,
            compute_score: self.hardware.logical_cores as f64,
            network_score: 100.0,
            local: true,
        }];
        // Runtime currently distributes to one paired worker. Matching that limit
        // keeps the Hub recommendation identical to what StartModel can execute.
        if let Some(worker) = self
            .nodes
            .iter()
            .find(|node| node.id != self.settings.node_id && node.is_usable_worker())
        {
            nodes.push(MemoryNode {
                name: worker.name.clone(),
                safe_memory_bytes: worker.available_memory_bytes,
                compute_score: 1.0,
                network_score: 50.0,
                local: false,
            });
        }
        nodes
    }

    pub fn installed_rating(&self, model: &ModelInfo) -> FitRating {
        rate_installed(model.file_size_bytes, &self.recommendation_nodes())
    }

    pub fn apply_hub_event(&mut self, event: HubEvent) {
        match event {
            HubEvent::SearchStarted(query) => {
                self.models.active_query = query;
                self.models.searching = true;
            }
            HubEvent::SearchFinished {
                query,
                repositories,
            } => {
                if query == self.models.active_query {
                    self.models.repositories = repositories;
                    self.models.repository_index = 0;
                    self.models.searching = false;
                    if self.models.repositories.is_empty() {
                        self.models.pane = ModelsPane::Search;
                        self.models.status = Some(
                            self.language
                                .text(
                                    "No GGUF repositories found. Try another search query.",
                                    "Репозитории GGUF не найдены. Попробуйте другой запрос.",
                                )
                                .into(),
                        );
                    } else {
                        self.models.pane = ModelsPane::Repositories;
                        self.models.status = None;
                    }
                }
            }
            HubEvent::RepositoryLoaded { repository, files } => {
                if self.models.open_repository.as_deref() == Some(&repository) {
                    self.models.files = files;
                    self.models.file_index = self
                        .models
                        .files
                        .iter()
                        .position(|file| file.rating == FitRating::Recommended)
                        .unwrap_or(0);
                    self.models.pane = ModelsPane::Files;
                    self.models.status = None;
                }
            }
            HubEvent::DownloadProgress(progress) => {
                self.models.download = Some(progress);
                self.models.status = None;
            }
            HubEvent::DownloadFinished(model) => {
                self.models.download = None;
                self.models
                    .installed
                    .retain(|existing| existing.path != model.path);
                self.models.installed.push(model);
                self.models
                    .installed
                    .sort_by(|left, right| left.name.cmp(&right.name));
                self.models.installed_index = self.models.installed.len().saturating_sub(1);
                self.models.pane = ModelsPane::Installed;
                self.models.status = Some(
                    self.language
                        .text(
                            "Download complete; press Enter to use",
                            "Загрузка завершена; Enter — использовать",
                        )
                        .into(),
                );
            }
            HubEvent::DownloadCancelled(_) => {
                self.models.download = None;
                self.models.status = Some(
                    self.language
                        .text(
                            "Download cancelled; partial file kept for resume",
                            "Загрузка отменена; частичный файл сохранён для продолжения",
                        )
                        .into(),
                );
            }
            HubEvent::ModelDeleted(path) => {
                self.models.installed.retain(|model| model.path != path);
                self.models.installed_index = self
                    .models
                    .installed_index
                    .min(self.models.installed.len().saturating_sub(1));
                if self.model.as_ref().is_some_and(|model| model.path == path) {
                    self.model = None;
                    self.settings.last_model = None;
                    let _ = self.settings.save(&self.paths);
                }
                self.models.status =
                    Some(self.language.text("Model deleted", "Модель удалена").into());
            }
            HubEvent::Error(message) => {
                self.models.searching = false;
                self.models.download = None;
                if message.contains("timed out")
                    || message.contains("Connect")
                    || message.contains("dns error")
                {
                    self.models.status = Some(
                        self.language
                            .text(
                                "Connection to Hugging Face timed out. Check network or proxy.",
                                "Таймаут подключения к Hugging Face. Проверьте интернет или прокси.",
                            )
                            .into(),
                    );
                } else {
                    self.models.status = Some(message);
                }
            }
        }
    }

    pub fn take_hub_command(&mut self) -> Option<HubCommand> {
        if self.cluster_running {
            None
        } else {
            self.models.delete_after_stop.take().map(HubCommand::Delete)
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
                        available_memory_bytes: node.memory_bytes,
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
                self.models.delete_after_stop = None;
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
            models: ModelsState {
                pane: ModelsPane::Search,
                search_input: String::new(),
                active_query: String::new(),
                searching: false,
                repositories: Vec::new(),
                repository_index: 0,
                open_repository: None,
                files: Vec::new(),
                file_index: 0,
                installed: Vec::new(),
                installed_index: 0,
                download: None,
                status: None,
                delete_confirmation: None,
                delete_after_stop: None,
            },
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
    fn models_search_is_emitted_as_a_background_hub_command() {
        let mut app = test_app();
        app.screen_index = 2;
        for character in "Qwen uncensored".chars() {
            app.handle_key(make_key(KeyCode::Char(character), KeyModifiers::NONE));
        }
        match app.handle_key(make_key(KeyCode::Enter, KeyModifiers::NONE)) {
            AppAction::Hub(HubCommand::Search(query)) => assert_eq!(query, "Qwen uncensored"),
            _ => panic!("expected an asynchronous Hub search"),
        }
        assert!(app.models.searching);
    }

    #[test]
    fn q_types_into_search_input_even_when_empty() {
        let mut app = test_app();
        app.screen_index = 2;
        app.models.pane = ModelsPane::Search;
        app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.running);
        assert_eq!(app.models.search_input, "q");

        let mut app = test_app();
        app.screen_index = 2;
        app.models.pane = ModelsPane::Search;
        app.models.search_input = "model".into();
        app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.running);
        assert_eq!(app.models.search_input, "modelq");

        let mut app = test_app();
        app.screen_index = 2;
        app.models.pane = ModelsPane::Installed;
        app.handle_key(make_key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.running);
    }

    #[test]
    fn selecting_an_installed_model_persists_it() {
        use std::io::Write;
        let directory =
            std::env::temp_dir().join(format!("bution-model-select-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("Qwen-Q4_K_M.gguf");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"GGUF\x03\x00\x00\x00").unwrap();
        let model = ModelInfo::inspect(&path).unwrap();
        let mut app = test_app();
        app.paths.data_dir = directory.clone();
        app.paths.models_dir = directory.clone();
        app.paths.settings_file = directory.join("settings.toml");
        app.paths.cache_dir = directory.join("cache");
        app.screen_index = 2;
        app.models.pane = ModelsPane::Installed;
        app.models.installed.push(model.clone());
        app.handle_key(make_key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.model.as_ref().unwrap().path, path);
        assert_eq!(
            Settings::load_or_create(&app.paths).unwrap().last_model,
            Some(path)
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn active_model_deletion_requires_confirmation_and_stop() {
        use std::io::Write;
        let directory =
            std::env::temp_dir().join(format!("bution-model-delete-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("Qwen-Q4_K_M.gguf");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"GGUF\x03\x00\x00\x00").unwrap();
        let model = ModelInfo::inspect(&path).unwrap();
        let mut app = test_app();
        app.screen_index = 2;
        app.models.pane = ModelsPane::Installed;
        app.models.installed.push(model.clone());
        app.model = Some(model);
        app.cluster_running = true;
        assert!(matches!(
            app.handle_key(make_key(KeyCode::Char('D'), KeyModifiers::NONE)),
            AppAction::None
        ));
        assert!(app.models.delete_after_stop.is_none());
        assert!(matches!(
            app.handle_key(make_key(KeyCode::Char('D'), KeyModifiers::NONE)),
            AppAction::Runtime(RuntimeCommand::StopModel)
        ));
        assert!(app.take_hub_command().is_none());
        app.apply_runtime_event(RuntimeEvent::ClusterStopped);
        assert!(
            matches!(app.take_hub_command(), Some(HubCommand::Delete(target)) if target == path)
        );
        std::fs::remove_dir_all(directory).unwrap();
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

    #[test]
    fn empty_search_results_keeps_search_pane_active_and_editable() {
        let mut app = test_app();
        app.screen_index = 2;
        app.models.pane = ModelsPane::Search;

        // Type query 'nonexistent'
        for c in "nonexistent".chars() {
            app.handle_key(make_key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        assert_eq!(app.models.search_input, "nonexistent");

        // Submit search
        match app.handle_key(make_key(KeyCode::Enter, KeyModifiers::NONE)) {
            AppAction::Hub(HubCommand::Search(query)) => assert_eq!(query, "nonexistent"),
            _ => panic!("expected search action"),
        }
        // Receive empty results
        app.apply_hub_event(HubEvent::SearchStarted("nonexistent".into()));
        app.apply_hub_event(HubEvent::SearchFinished {
            query: "nonexistent".into(),
            repositories: Vec::new(),
        });

        // Must stay in Search pane and show status
        assert_eq!(app.models.pane, ModelsPane::Search);
        assert!(!app.models.searching);
        assert!(app.models.status.is_some());

        // Can immediately backspace and type new characters
        app.handle_key(make_key(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(make_key(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.models.search_input, "nonexisten2");
    }
}
