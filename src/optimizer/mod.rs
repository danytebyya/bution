//! Memory-safe tensor distribution across local and RPC devices.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
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
}
