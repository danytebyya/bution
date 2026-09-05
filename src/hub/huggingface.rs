//! Minimal typed client for the public Hugging Face Hub API.

use crate::hub::filtering::primary_quantization;
use crate::hub::quantization::Quantization;
use anyhow::{Context, Result, bail};
use reqwest::{Client, Url};
use serde::Deserialize;
use std::time::Duration;

const ENDPOINT: &str = "https://huggingface.co";

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

#[derive(Debug, Deserialize)]
struct ApiModel {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    sha: String,
    #[serde(default)]
    siblings: Vec<ApiSibling>,
}

#[derive(Debug, Deserialize)]
struct ApiSibling {
    rfilename: String,
    size: Option<u64>,
    lfs: Option<ApiLfs>,
}

#[derive(Debug, Deserialize)]
struct ApiLfs {
    size: u64,
}

impl HuggingFaceClient {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .user_agent(format!("BUTION/{}", env!("CARGO_PKG_VERSION")))
                .connect_timeout(Duration::from_secs(10))
                .build()?,
            endpoint: Url::parse(ENDPOINT)?,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_endpoint(endpoint: Url) -> Result<Self> {
        Ok(Self {
            client: Client::builder().build()?,
            endpoint,
        })
    }

    pub async fn search(&self, query: &str) -> Result<Vec<HubRepository>> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let mut url = self.endpoint.join("api/models")?;
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
            .filter(|model| {
                model
                    .siblings
                    .iter()
                    .any(|file| file.rfilename.to_ascii_lowercase().ends_with(".gguf"))
            })
            .map(|model| HubRepository {
                id: model.id,
                downloads: model.downloads,
                likes: model.likes,
            })
            .collect())
    }

    pub async fn files(&self, repository: &str) -> Result<Vec<HubFile>> {
        let mut url = self.endpoint.clone();
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
        let revision = if model.sha.is_empty() {
            "main".to_owned()
        } else {
            model.sha
        };
        let mut files = model
            .siblings
            .into_iter()
            .filter_map(|file| {
                let quantization = primary_quantization(&file.rfilename)?;
                let size_bytes = file.size.or_else(|| file.lfs.map(|lfs| lfs.size))?;
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
