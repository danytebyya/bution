use super::OptimizationResult;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizationFingerprint {
    pub model: String,
    pub hardware: Vec<String>,
    pub network: Vec<String>,
}

impl OptimizationFingerprint {
    pub fn cache_key(&self) -> Result<String> {
        let encoded = serde_json::to_vec(self).context("could not fingerprint optimization")?;
        let digest = Sha256::digest(encoded);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

#[derive(Debug, Clone)]
pub struct OptimizationCache {
    directory: PathBuf,
}

impl OptimizationCache {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn load(
        &self,
        fingerprint: &OptimizationFingerprint,
    ) -> Result<Option<OptimizationResult>> {
        let path = self.path(fingerprint)?;
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(&path)
            .with_context(|| format!("could not read optimization cache {}", path.display()))?;
        let result = serde_json::from_slice(&bytes).context("optimization cache is invalid")?;
        Ok(Some(result))
    }

    pub fn save(
        &self,
        fingerprint: &OptimizationFingerprint,
        result: &OptimizationResult,
    ) -> Result<()> {
        fs::create_dir_all(&self.directory).context("could not create optimization cache")?;
        let path = self.path(fingerprint)?;
        let temporary = path.with_extension("tmp");
        let bytes =
            serde_json::to_vec_pretty(result).context("could not serialize optimization")?;
        fs::write(&temporary, bytes).context("could not write optimization cache")?;
        fs::rename(&temporary, &path).context("could not replace optimization cache")
    }

    fn path(&self, fingerprint: &OptimizationFingerprint) -> Result<PathBuf> {
        Ok(self
            .directory
            .join(format!("{}.json", fingerprint.cache_key()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::LlamaBenchmark;
    use crate::optimizer::OptimizationTrial;
    use uuid::Uuid;

    #[test]
    fn saves_results_for_exact_cluster_fingerprint() {
        let directory = std::env::temp_dir().join(format!("bution-cache-{}", Uuid::new_v4()));
        let cache = OptimizationCache::new(directory.clone());
        let fingerprint = OptimizationFingerprint {
            model: "qwen:1234".into(),
            hardware: vec!["mac:m1:16".into(), "pc:ryzen:16".into()],
            network: vec!["ethernet:934:0.9".into()],
        };
        let result = OptimizationResult {
            trials: vec![OptimizationTrial {
                tensor_split: vec![0.7, 0.3],
                benchmark: LlamaBenchmark {
                    prompt_tokens_per_second: 800.0,
                    generation_tokens_per_second: 8.2,
                    estimated_ttft_ms: 700.0,
                    compute_score: 10.0,
                },
            }],
            best_index: 0,
        };
        cache.save(&fingerprint, &result).unwrap();
        assert_eq!(cache.load(&fingerprint).unwrap(), Some(result));
        fs::remove_dir_all(directory).unwrap();
    }
}
