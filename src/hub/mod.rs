//! Hugging Face GGUF discovery, filtering, recommendations, and background downloads.

pub mod download;
pub mod filtering;
pub mod huggingface;
pub mod quantization;
pub mod recommendations;

use crate::models::ModelInfo;
use anyhow::Result;
use download::{DownloadProgress, download_file};
use huggingface::{HubFile, HubRepository, HuggingFaceClient};
use recommendations::{MemoryNode, RankedFile, rank_files};
use std::path::PathBuf;
use tokio::sync::{mpsc, watch};

#[derive(Debug)]
pub enum HubCommand {
    Search(String),
    OpenRepository(String, Vec<MemoryNode>),
    Download(HubFile),
    CancelDownload,
    Delete(PathBuf),
}

#[derive(Debug)]
pub enum HubEvent {
    SearchStarted(String),
    SearchFinished {
        query: String,
        repositories: Vec<HubRepository>,
    },
    RepositoryLoaded {
        repository: String,
        files: Vec<RankedFile>,
    },
    DownloadProgress(DownloadProgress),
    DownloadFinished(ModelInfo),
    DownloadCancelled(PathBuf),
    ModelDeleted(PathBuf),
    Error(String),
}

pub struct HubHandle {
    pub events: mpsc::Receiver<HubEvent>,
    commands: mpsc::Sender<HubCommand>,
}

impl HubHandle {
    pub fn start(models_dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&models_dir)?;
        let client = HuggingFaceClient::new()?;
        let (commands, command_receiver) = mpsc::channel(32);
        let (events_sender, events) = mpsc::channel(128);
        tokio::spawn(run_loop(
            client,
            models_dir,
            command_receiver,
            events_sender,
        ));
        Ok(Self { events, commands })
    }

    pub async fn command(&self, command: HubCommand) -> Result<()> {
        self.commands
            .send(command)
            .await
            .map_err(|_| anyhow::anyhow!("model hub background task has stopped"))
    }
}

async fn run_loop(
    client: HuggingFaceClient,
    models_dir: PathBuf,
    mut commands: mpsc::Receiver<HubCommand>,
    events: mpsc::Sender<HubEvent>,
) {
    let mut cancellation: Option<watch::Sender<bool>> = None;
    while let Some(command) = commands.recv().await {
        match command {
            HubCommand::Search(query) => {
                let client = client.clone();
                let sender = events.clone();
                tokio::spawn(async move {
                    let _ = sender.send(HubEvent::SearchStarted(query.clone())).await;
                    match client.search(&query).await {
                        Ok(repositories) => {
                            let _ = sender
                                .send(HubEvent::SearchFinished {
                                    query,
                                    repositories,
                                })
                                .await;
                        }
                        Err(error) => {
                            let _ = sender.send(HubEvent::Error(format!("{error:#}"))).await;
                        }
                    }
                });
            }
            HubCommand::OpenRepository(repository, nodes) => {
                let client = client.clone();
                let sender = events.clone();
                tokio::spawn(async move {
                    match client.files(&repository).await {
                        Ok(files) => {
                            let files = rank_files(files, &nodes);
                            let _ = sender
                                .send(HubEvent::RepositoryLoaded { repository, files })
                                .await;
                        }
                        Err(error) => {
                            let _ = sender.send(HubEvent::Error(format!("{error:#}"))).await;
                        }
                    }
                });
            }
            HubCommand::Download(file) => {
                if let Some(previous) = cancellation.take() {
                    let _ = previous.send(true);
                }
                let (cancel, receiver) = watch::channel(false);
                cancellation = Some(cancel);
                let sender = events.clone();
                let directory = models_dir.clone();
                let client = client.clone();
                tokio::spawn(async move {
                    match download_file(&client, &file, &directory, receiver, &sender).await {
                        Ok(download::DownloadOutcome::Finished(model)) => {
                            let _ = sender.send(HubEvent::DownloadFinished(model)).await;
                        }
                        Ok(download::DownloadOutcome::Cancelled(path)) => {
                            let _ = sender.send(HubEvent::DownloadCancelled(path)).await;
                        }
                        Err(error) => {
                            let _ = sender.send(HubEvent::Error(format!("{error:#}"))).await;
                        }
                    }
                });
            }
            HubCommand::CancelDownload => {
                if let Some(cancel) = cancellation.take() {
                    let _ = cancel.send(true);
                }
            }
            HubCommand::Delete(path) => {
                let sender = events.clone();
                let directory = models_dir.clone();
                tokio::spawn(async move {
                    let allowed = path.parent().is_some_and(|parent| parent == directory)
                        && path.extension().and_then(|value| value.to_str()) == Some("gguf");
                    if !allowed {
                        let _ = sender
                            .send(HubEvent::Error(
                                "refusing to delete a model outside the BUTION models directory"
                                    .into(),
                            ))
                            .await;
                        return;
                    }
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => {
                            let _ = sender.send(HubEvent::ModelDeleted(path)).await;
                        }
                        Err(error) => {
                            let _ = sender
                                .send(HubEvent::Error(format!("could not delete model: {error}")))
                                .await;
                        }
                    }
                });
            }
        }
    }
    if let Some(cancel) = cancellation {
        let _ = cancel.send(true);
    }
}
