//! Memory-safe tensor distribution across local and RPC devices.

mod cache;

pub use cache::{OptimizationCache, OptimizationFingerprint};

use crate::benchmark::{LlamaBenchmark, run_llama_benchmark};
use crate::llama::{BenchConfig, LlamaBinaries};
use crate::processes::ProcessManager;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeCapacity {
    pub node_id: Uuid,
    pub name: String,
    pub available_memory_bytes: u64,
    /// Relative compute performance; benchmark tok/s is a suitable value.
    pub compute_score: f64,
    /// Network score in the range 0..100. Ignored for the local node.
    pub network_score: f64,
    pub local: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Allocation {
    pub node_id: Uuid,
    pub name: String,
    pub fraction: f64,
    pub estimated_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistributionPlan {
    pub model_memory_bytes: u64,
    pub allocations: Vec<Allocation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationTrial {
    pub tensor_split: Vec<f32>,
    pub benchmark: LlamaBenchmark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizationResult {
    pub trials: Vec<OptimizationTrial>,
    pub best_index: usize,
}

impl OptimizationResult {
    pub fn best(&self) -> &OptimizationTrial {
        &self.trials[self.best_index]
    }
}

impl DistributionPlan {
    pub fn tensor_split(&self) -> Vec<f32> {
        self.allocations
            .iter()
            .map(|allocation| allocation.fraction as f32)
            .collect()
    }
}

pub fn plan_distribution(
    model_memory_bytes: u64,
    nodes: &[NodeCapacity],
) -> Result<DistributionPlan> {
    if model_memory_bytes == 0 {
        bail!("model memory requirement must be greater than zero");
    }
    if nodes.is_empty() {
        bail!("at least one node is required");
    }
    let total_memory = nodes.iter().fold(0_u64, |total, node| {
        total.saturating_add(node.available_memory_bytes)
    });
    if total_memory < model_memory_bytes {
        bail!("cluster memory is insufficient for the selected model");
    }

    let best_compute = nodes
        .iter()
        .map(|node| node.compute_score)
        .filter(|score| score.is_finite() && *score > 0.0)
        .fold(1.0_f64, f64::max);
    let priorities: Vec<f64> = nodes
        .iter()
        .map(|node| {
            let memory = node.available_memory_bytes as f64 / model_memory_bytes as f64;
            let compute = if node.compute_score.is_finite() {
                (node.compute_score.max(0.0) / best_compute).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let network = if node.local {
                1.0
            } else {
                (node.network_score / 100.0).clamp(0.0, 1.0)
            };
            memory * (0.35 + 0.65 * compute) * (0.45 + 0.55 * network)
        })
        .collect();
    let capacities: Vec<f64> = nodes
        .iter()
        .map(|node| node.available_memory_bytes as f64 / model_memory_bytes as f64)
        .collect();
    let fractions = capped_weighted_distribution(&priorities, &capacities)?;
    let allocations = nodes
        .iter()
        .zip(fractions)
        .filter(|(_, fraction)| *fraction > 0.000_001)
        .map(|(node, fraction)| Allocation {
            node_id: node.node_id,
            name: node.name.clone(),
            fraction,
            estimated_bytes: (model_memory_bytes as f64 * fraction).ceil() as u64,
        })
        .collect();
    Ok(DistributionPlan {
        model_memory_bytes,
        allocations,
    })
}

/// MVP search space for two nodes. Invalid splits that exceed a node's memory
/// budget are removed before expensive llama-bench runs.
pub fn two_node_candidates(model_memory_bytes: u64, nodes: &[NodeCapacity]) -> Vec<Vec<f32>> {
    if nodes.len() != 2 || model_memory_bytes == 0 {
        return Vec::new();
    }
    [[0.8_f32, 0.2_f32], [0.7, 0.3], [0.6, 0.4], [0.5, 0.5]]
        .into_iter()
        .filter(|split| {
            split.iter().zip(nodes).all(|(fraction, node)| {
                model_memory_bytes as f64 * *fraction as f64 <= node.available_memory_bytes as f64
            })
        })
        .map(Vec::from)
        .collect()
}

pub async fn optimize_cluster(
    manager: &mut ProcessManager,
    binaries: &LlamaBinaries,
    base_config: &BenchConfig,
    candidates: &[Vec<f32>],
    timeout_per_trial: Duration,
) -> Result<OptimizationResult> {
    let mut trials = Vec::new();
    for split in candidates {
        let mut config = base_config.clone();
        config.tensor_split = split.clone();
        if let Ok(benchmark) =
            run_llama_benchmark(manager, binaries, &config, timeout_per_trial).await
        {
            trials.push(OptimizationTrial {
                tensor_split: split.clone(),
                benchmark,
            });
        }
    }
    select_best_trials(trials)
}

pub fn select_best_trials(trials: Vec<OptimizationTrial>) -> Result<OptimizationResult> {
    if trials.is_empty() {
        bail!("none of the cluster distributions completed successfully");
    }
    let best_index = trials
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| {
            left.benchmark
                .generation_tokens_per_second
                .partial_cmp(&right.benchmark.generation_tokens_per_second)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    right
                        .benchmark
                        .estimated_ttft_ms
                        .partial_cmp(&left.benchmark.estimated_ttft_ms)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .map(|(index, _)| index)
        .expect("non-empty trials");
    Ok(OptimizationResult { trials, best_index })
}

fn capped_weighted_distribution(priorities: &[f64], capacities: &[f64]) -> Result<Vec<f64>> {
    let mut output = vec![0.0; priorities.len()];
    let mut active: Vec<usize> = (0..priorities.len()).collect();
    let mut remaining = 1.0;

    while remaining > 1e-9 && !active.is_empty() {
        let weight_sum: f64 = active.iter().map(|index| priorities[*index]).sum();
        let equal_share = weight_sum <= f64::EPSILON;
        let mut capped = Vec::new();
        for &index in &active {
            let share = if equal_share {
                remaining / active.len() as f64
            } else {
                remaining * priorities[index] / weight_sum
            };
            let available = (capacities[index] - output[index]).max(0.0);
            if share >= available - 1e-12 {
                output[index] += available;
                remaining -= available;
                capped.push(index);
            }
        }
        if capped.is_empty() {
            for &index in &active {
                output[index] += if equal_share {
                    remaining / active.len() as f64
                } else {
                    remaining * priorities[index] / weight_sum
                };
            }
            remaining = 0.0;
        } else {
            active.retain(|index| !capped.contains(index));
        }
    }

    if remaining > 1e-6 {
        bail!("node memory caps cannot satisfy the model allocation");
    }
    let total: f64 = output.iter().sum();
    if total > 0.0 {
        for fraction in &mut output {
            *fraction /= total;
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::GIB;

    fn node(name: &str, memory: u64, compute: f64, network: f64, local: bool) -> NodeCapacity {
        NodeCapacity {
            node_id: Uuid::new_v4(),
            name: name.into(),
            available_memory_bytes: memory,
            compute_score: compute,
            network_score: network,
            local,
        }
    }

    #[test]
    fn faster_main_receives_more_than_half() {
        let nodes = [
            node("MacBook", 16 * GIB, 12.0, 100.0, true),
            node("HONOR", 16 * GIB, 5.0, 75.0, false),
        ];
        let plan = plan_distribution(24 * GIB, &nodes).unwrap();
        assert!(plan.allocations[0].fraction > 0.5);
        assert!(plan.allocations[0].estimated_bytes <= 16 * GIB);
        assert!(plan.allocations[1].estimated_bytes <= 16 * GIB);
    }

    #[test]
    fn memory_cap_overrides_compute_preference() {
        let nodes = [
            node("Fast", 8 * GIB, 100.0, 100.0, true),
            node("Large", 24 * GIB, 1.0, 50.0, false),
        ];
        let plan = plan_distribution(24 * GIB, &nodes).unwrap();
        assert!(plan.allocations[0].fraction <= 1.0 / 3.0 + 1e-9);
    }

    #[test]
    fn rejects_model_larger_than_cluster() {
        let nodes = [node("Only", 8 * GIB, 1.0, 100.0, true)];
        assert!(plan_distribution(9 * GIB, &nodes).is_err());
    }

    #[test]
    fn filters_splits_that_exceed_memory() {
        let nodes = [
            node("Mac", 14 * GIB, 1.0, 100.0, true),
            node("PC", 14 * GIB, 1.0, 100.0, false),
        ];
        let candidates = two_node_candidates(24 * GIB, &nodes);
        assert_eq!(candidates, vec![vec![0.5, 0.5]]);
    }

    #[test]
    fn selects_highest_generation_throughput() {
        let trials = [6.7, 8.2, 7.9]
            .into_iter()
            .enumerate()
            .map(|(index, speed)| OptimizationTrial {
                tensor_split: vec![0.8 - index as f32 * 0.1, 0.2 + index as f32 * 0.1],
                benchmark: LlamaBenchmark {
                    prompt_tokens_per_second: 100.0,
                    generation_tokens_per_second: speed,
                    estimated_ttft_ms: 1_000.0,
                    compute_score: speed,
                },
            })
            .collect();
        let result = select_best_trials(trials).unwrap();
        assert_eq!(result.best().benchmark.generation_tokens_per_second, 8.2);
        assert_eq!(result.best().tensor_split, vec![0.7, 0.3]);
    }
}
