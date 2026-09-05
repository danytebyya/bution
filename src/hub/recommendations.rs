//! Memory-aware GGUF ranking using the same distribution planner as inference.

use crate::hub::huggingface::HubFile;
use crate::models::estimate_memory;
use crate::optimizer::{NodeCapacity, plan_distribution};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryNode {
    pub name: String,
    pub safe_memory_bytes: u64,
    pub compute_score: f64,
    pub network_score: f64,
    pub local: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FitRating {
    Recommended,
    Fits,
    Tight,
    TooLarge,
}

impl FitRating {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recommended => "★ Recommended",
            Self::Fits => "Fits",
            Self::Tight => "Tight",
            Self::TooLarge => "Too large",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RankedFile {
    pub file: HubFile,
    pub estimated_memory_bytes: u64,
    pub rating: FitRating,
}

pub fn rank_files(files: Vec<HubFile>, nodes: &[MemoryNode]) -> Vec<RankedFile> {
    let capacities = nodes
        .iter()
        .map(|node| NodeCapacity {
            node_id: Uuid::new_v4(),
            name: node.name.clone(),
            available_memory_bytes: node.safe_memory_bytes,
            compute_score: node.compute_score,
            network_score: node.network_score,
            local: node.local,
        })
        .collect::<Vec<_>>();
    let total = nodes.iter().fold(0_u64, |sum, node| {
        sum.saturating_add(node.safe_memory_bytes)
    });
    let mut ranked = files
        .into_iter()
        .map(|file| {
            let required = estimate_memory(file.size_bytes);
            let rating = match plan_distribution(required, &capacities) {
                Err(_) => FitRating::TooLarge,
                Ok(plan) => {
                    let cluster_pressure = required as f64 / total.max(1) as f64;
                    let node_pressure = plan
                        .allocations
                        .iter()
                        .filter_map(|allocation| {
                            nodes
                                .iter()
                                .find(|node| node.name == allocation.name)
                                .map(|node| {
                                    allocation.estimated_bytes as f64
                                        / node.safe_memory_bytes.max(1) as f64
                                })
                        })
                        .fold(0.0_f64, f64::max);
                    if cluster_pressure <= 0.82 && node_pressure <= 0.88 {
                        FitRating::Fits
                    } else {
                        FitRating::Tight
                    }
                }
            };
            RankedFile {
                file,
                estimated_memory_bytes: required,
                rating,
            }
        })
        .collect::<Vec<_>>();

    // File size is a reliable cross-family proxy for retained precision. Recommend
    // the highest-quality option with comfortable runtime/KV headroom.
    if let Some(best) = ranked
        .iter_mut()
        .filter(|entry| entry.rating == FitRating::Fits)
        .max_by_key(|entry| entry.file.size_bytes)
    {
        best.rating = FitRating::Recommended;
    }
    ranked.sort_by_key(|entry| entry.file.size_bytes);
    ranked
}

pub fn rate_installed(file_size_bytes: u64, nodes: &[MemoryNode]) -> FitRating {
    let required = estimate_memory(file_size_bytes);
    let capacities = nodes
        .iter()
        .map(|node| NodeCapacity {
            node_id: Uuid::new_v4(),
            name: node.name.clone(),
            available_memory_bytes: node.safe_memory_bytes,
            compute_score: node.compute_score,
            network_score: node.network_score,
            local: node.local,
        })
        .collect::<Vec<_>>();
    if plan_distribution(required, &capacities).is_err() {
        FitRating::TooLarge
    } else {
        let total = nodes.iter().fold(0_u64, |sum, node| {
            sum.saturating_add(node.safe_memory_bytes)
        });
        if required as f64 / total.max(1) as f64 <= 0.82 {
            FitRating::Recommended
        } else {
            FitRating::Tight
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::GIB;
    use crate::hub::quantization::Quantization;

    fn file(name: &str, gib: u64) -> HubFile {
        HubFile {
            repository: "owner/model-GGUF".into(),
            revision: "main".into(),
            filename: format!("model-{name}.gguf"),
            size_bytes: gib * GIB,
            quantization: Quantization(name.into()),
        }
    }

    #[test]
    fn recommends_largest_comfortable_quant_and_keeps_all_options() {
        let nodes = [MemoryNode {
            name: "Local".into(),
            safe_memory_bytes: 24 * GIB,
            compute_score: 8.0,
            network_score: 100.0,
            local: true,
        }];
        let result = rank_files(
            vec![file("Q4_K_M", 12), file("Q5_K_M", 15), file("Q8_0", 25)],
            &nodes,
        );
        assert_eq!(result.len(), 3);
        assert_eq!(result[1].rating, FitRating::Recommended);
        assert_eq!(result[2].rating, FitRating::TooLarge);
    }

    #[test]
    fn two_nodes_use_cluster_memory_without_exceeding_individual_caps() {
        let nodes = [
            MemoryNode {
                name: "Local".into(),
                safe_memory_bytes: 10 * GIB,
                compute_score: 10.0,
                network_score: 100.0,
                local: true,
            },
            MemoryNode {
                name: "Worker".into(),
                safe_memory_bytes: 14 * GIB,
                compute_score: 5.0,
                network_score: 70.0,
                local: false,
            },
        ];
        let result = rank_files(vec![file("Q4_K_M", 15), file("Q8_0", 23)], &nodes);
        assert_ne!(result[0].rating, FitRating::TooLarge);
        assert_eq!(result[1].rating, FitRating::TooLarge);
    }
}
