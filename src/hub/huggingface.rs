//! Minimal typed client for the public Hugging Face Hub API.

use crate::hub::filtering::primary_quantization;
use crate::hub::quantization::Quantization;
use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::time::Duration;

const DEFAULT_ENDPOINT: &str = "https://huggingface.co";
const FALLBACK_ENDPOINT: &str = "https://hf-mirror.com";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubRepository {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubFile {
    pub repository: String,
    pub revision: String,
    pub filename: String,
    pub size_bytes: u64,
    pub quantization: Quantization,
}

#[derive(Debug, Clone)]
pub struct HuggingFaceClient {
    client: Client,
    endpoint: Url,
}

#[derive(Debug, Deserialize, Default)]
struct ApiModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "_id")]
    underscore_id: Option<String>,
    #[serde(default)]
    downloads: Option<u64>,
    #[serde(default)]
    likes: Option<u64>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    siblings: Vec<ApiSibling>,
}

#[derive(Debug, Deserialize, Default)]
struct ApiSibling {
    #[serde(default)]
    rfilename: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<ApiLfs>,
}

#[derive(Debug, Deserialize, Default)]
struct ApiLfs {
    #[serde(default)]
    size: Option<u64>,
}

impl HuggingFaceClient {
    pub fn new() -> Result<Self> {
        let endpoint_str =
            std::env::var("HF_ENDPOINT").unwrap_or_else(|_| DEFAULT_ENDPOINT.to_string());
        Ok(Self {
            client: Client::builder()
                .user_agent(format!("BUTION/{}", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(45))
                .pool_idle_timeout(Duration::from_secs(90))
                .tcp_keepalive(Duration::from_secs(30))
                .build()?,
            endpoint: Url::parse(&endpoint_str)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_endpoint(endpoint: Url) -> Result<Self> {
        Ok(Self {
            client: Client::builder().build()?,
            endpoint,
        })
    }

    async fn search_endpoint(&self, endpoint: &Url, query: &str) -> Result<Vec<HubRepository>> {
        let mut url = endpoint.join("api/models")?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("filter", "gguf")
            .append_pair("sort", "downloads")
            .append_pair("direction", "-1")
            .append_pair("limit", "30")
            .append_pair("full", "true");
        let response = self
            .client
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?;
        let models: Vec<ApiModel> = response
            .json()
            .await
            .context("invalid Hugging Face search response")?;
        Ok(models
            .into_iter()
            .filter_map(|model| {
                let id = model.id.or(model.underscore_id)?;
                let has_gguf = model
                    .siblings
                    .iter()
                    .any(|file| file.rfilename.to_ascii_lowercase().ends_with(".gguf"));
                if !has_gguf {
                    return None;
                }
                Some(HubRepository {
                    id,
                    downloads: model.downloads.unwrap_or(0),
                    likes: model.likes.unwrap_or(0),
                })
            })
            .collect())
    }

    pub async fn search(&self, query: &str) -> Result<Vec<HubRepository>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        match self.search_endpoint(&self.endpoint, query).await {
            Ok(repos) => Ok(repos),
            Err(primary_err) => {
                let base = self.endpoint.as_str().trim_end_matches('/');
                if base == DEFAULT_ENDPOINT {
                    if let Ok(fallback_url) = Url::parse(FALLBACK_ENDPOINT) {
                        if let Ok(repos) = self.search_endpoint(&fallback_url, query).await {
                            return Ok(repos);
                        }
                    }
                }
                Err(primary_err)
            }
        }
    }

    async fn files_endpoint(&self, endpoint: &Url, repository: &str) -> Result<Vec<HubFile>> {
        let mut url = endpoint.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("invalid Hub endpoint"))?;
            segments.extend(["api", "models"]);
            segments.extend(repository.split('/'));
        }
        url.query_pairs_mut().append_pair("blobs", "true");
        let response = self
            .client
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?;
        let model: ApiModel = response
            .json()
            .await
            .context("invalid Hugging Face repository response")?;
        let revision = model.sha.unwrap_or_else(|| "main".to_owned());
        let mut files = model
            .siblings
            .into_iter()
            .filter_map(|file| {
                if file.rfilename.is_empty() {
                    return None;
                }
                let quantization = primary_quantization(&file.rfilename)?;
                let size_bytes = file.size.or_else(|| file.lfs.and_then(|lfs| lfs.size))?;
                (size_bytes > 0).then(|| HubFile {
                    repository: repository.to_owned(),
                    revision: revision.clone(),
                    filename: file.rfilename,
                    size_bytes,
                    quantization,
                })
            })
            .collect::<Vec<_>>();
        files.sort_by_key(|file| file.size_bytes);
        if files.is_empty() {
            bail!("repository contains no supported standalone GGUF files");
        }
        Ok(files)
    }

    pub async fn files(&self, repository: &str) -> Result<Vec<HubFile>> {
        match self.files_endpoint(&self.endpoint, repository).await {
            Ok(files) => Ok(files),
            Err(primary_err) => {
                let base = self.endpoint.as_str().trim_end_matches('/');
                if base == DEFAULT_ENDPOINT {
                    if let Ok(fallback_url) = Url::parse(FALLBACK_ENDPOINT) {
                        if let Ok(files) = self.files_endpoint(&fallback_url, repository).await {
                            return Ok(files);
                        }
                    }
                }
                Err(primary_err)
            }
        }
    }

    pub fn download_url(&self, file: &HubFile) -> Result<Url> {
        let mut url = self.endpoint.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("invalid Hub endpoint"))?;
            segments.extend(file.repository.split('/'));
            segments.push("resolve");
            segments.push(&file.revision);
            segments.extend(file.filename.split('/'));
        }
        url.query_pairs_mut().append_pair("download", "true");
        Ok(url)
    }

    pub(crate) fn http(&self) -> &Client {
        &self.client
    }
}
