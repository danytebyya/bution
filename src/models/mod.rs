//! GGUF validation, model inventory, and memory fit recommendations.

use crate::hardware::GIB;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const GGUF_MAGIC: &[u8; 4] = b"GGUF";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub path: PathBuf,
    pub name: String,
    pub gguf_version: u32,
    pub file_size_bytes: u64,
    pub estimated_memory_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunRecommendation {
    Local,
    Cluster,
    InsufficientMemory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FitReport {
    pub model_required_bytes: u64,
    pub local_available_bytes: u64,
    pub cluster_available_bytes: u64,
    pub recommendation: RunRecommendation,
}

impl ModelInfo {
    pub fn inspect(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.extension().and_then(|extension| extension.to_str()) != Some("gguf") {
            bail!("the selected file is not a .gguf model");
        }
        let mut file = File::open(&path)
            .with_context(|| format!("could not open model {}", path.display()))?;
        let mut header = [0_u8; 8];
        file.read_exact(&mut header)
            .context("model is too small to contain a GGUF header")?;
        if &header[..4] != GGUF_MAGIC {
            bail!("the selected file does not have a GGUF header");
        }
        let gguf_version = u32::from_le_bytes(header[4..8].try_into().expect("four bytes"));
        if !(2..=3).contains(&gguf_version) {
            bail!("GGUF version {gguf_version} is not supported");
        }
        let file_size_bytes = file.metadata()?.len();
        let estimated_memory_bytes = estimate_memory(file_size_bytes);
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("GGUF model")
            .replace(['_', '-'], " ");
        Ok(Self {
            path,
            name,
            gguf_version,
            file_size_bytes,
            estimated_memory_bytes,
        })
    }

    pub fn fit_report(&self, local_available: u64, worker_available: &[u64]) -> FitReport {
        let cluster_available = worker_available
            .iter()
            .fold(local_available, |total, memory| {
                total.saturating_add(*memory)
            });
        let recommendation = if self.estimated_memory_bytes <= local_available {
            RunRecommendation::Local
        } else if self.estimated_memory_bytes <= cluster_available {
            RunRecommendation::Cluster
        } else {
            RunRecommendation::InsufficientMemory
        };
        FitReport {
            model_required_bytes: self.estimated_memory_bytes,
            local_available_bytes: local_available,
            cluster_available_bytes: cluster_available,
            recommendation,
        }
    }
}

/// Accounts for runtime tensors, allocator alignment, and a modest KV cache.
pub fn estimate_memory(file_size: u64) -> u64 {
    file_size
        .saturating_add(file_size.saturating_mul(13) / 100)
        .saturating_add(GIB)
}

pub fn scan_directory(directory: &Path) -> Result<Vec<ModelInfo>> {
    let mut models = Vec::new();
    if !directory.is_dir() {
        return Ok(models);
    }
    for entry in fs::read_dir(directory)
        .with_context(|| format!("could not scan model directory {}", directory.display()))?
    {
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("gguf") {
            if let Ok(model) = ModelInfo::inspect(path) {
                models.push(model);
            }
        }
    }
    models.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom, Write};
    use uuid::Uuid;

    fn fake_model(size: u64) -> PathBuf {
        let path = std::env::temp_dir().join(format!("Qwen-32B-Q4_K_M-{}.gguf", Uuid::new_v4()));
        let mut file = File::create(&path).unwrap();
        file.write_all(b"GGUF\x03\x00\x00\x00").unwrap();
        file.seek(SeekFrom::Start(size.saturating_sub(1))).unwrap();
        file.write_all(&[0]).unwrap();
        path
    }

    #[test]
    fn validates_gguf_and_estimates_memory() {
        let path = fake_model(20 * GIB);
        let model = ModelInfo::inspect(&path).unwrap();
        assert_eq!(model.gguf_version, 3);
        assert_eq!(model.estimated_memory_bytes, 23 * GIB + (GIB * 6 / 10));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn recommends_cluster_only_when_local_memory_is_insufficient() {
        let path = fake_model(18 * GIB);
        let model = ModelInfo::inspect(&path).unwrap();
        let report = model.fit_report(13 * GIB, &[13 * GIB]);
        assert_eq!(report.recommendation, RunRecommendation::Cluster);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_wrong_magic() {
        let path = std::env::temp_dir().join(format!("bad-{}.gguf", Uuid::new_v4()));
        fs::write(&path, b"NOPE\x03\x00\x00\x00").unwrap();
        assert!(ModelInfo::inspect(&path).is_err());
        fs::remove_file(path).unwrap();
    }
}
