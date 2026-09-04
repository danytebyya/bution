use super::{NetworkBenchmark, NetworkInterface, Stability};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasuredRoute {
    pub interface: NetworkInterface,
    pub remote_address: IpAddr,
    pub benchmark: NetworkBenchmark,
}

impl MeasuredRoute {
    pub fn score(&self) -> f64 {
        if !self.interface.usable_for_cluster() {
            return f64::NEG_INFINITY;
        }
        network_score(&self.benchmark)
    }
}

/// Weighted score tuned for llama.cpp RPC, where sustained throughput matters
/// most and high latency compounds token-generation stalls.
pub fn network_score(benchmark: &NetworkBenchmark) -> f64 {
    let bandwidth = (benchmark.bandwidth.megabits_per_second / 1_000.0).clamp(0.0, 1.0) * 55.0;
    let latency = 30.0 / (1.0 + benchmark.latency.average_ms.max(0.0) / 5.0);
    let stability = match benchmark.stability {
        Stability::Excellent => 15.0,
        Stability::Good => 9.0,
        Stability::Unstable => 0.0,
    } * benchmark.latency.success_rate.clamp(0.0, 1.0);
    bandwidth + latency + stability
}

pub fn select_best_route(routes: &[MeasuredRoute]) -> Option<&MeasuredRoute> {
    routes.iter().max_by(|left, right| {
        left.score()
            .partial_cmp(&right.score())
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{BandwidthStats, InterfaceKind, LatencyStats};

    fn route(kind: InterfaceKind, mbps: f64, latency_ms: f64) -> MeasuredRoute {
        MeasuredRoute {
            interface: NetworkInterface {
                name: format!("{kind:?}"),
                kind,
                address: "192.168.1.2".parse().unwrap(),
                prefix_len: 24,
                is_vpn: kind == InterfaceKind::Vpn,
            },
            remote_address: "192.168.1.3".parse().unwrap(),
            benchmark: NetworkBenchmark {
                latency: LatencyStats {
                    average_ms: latency_ms,
                    minimum_ms: latency_ms,
                    jitter_ms: 0.5,
                    success_rate: 1.0,
                },
                bandwidth: BandwidthStats {
                    megabits_per_second: mbps,
                    transferred_bytes: 1,
                    elapsed_seconds: 1.0,
                },
                stability: Stability::Excellent,
            },
        }
    }

    #[test]
    fn selects_fast_ethernet_over_wifi() {
        let routes = [
            route(InterfaceKind::Wifi, 286.0, 6.2),
            route(InterfaceKind::Ethernet, 934.0, 0.9),
        ];
        assert_eq!(
            select_best_route(&routes).unwrap().interface.kind,
            InterfaceKind::Ethernet
        );
    }

    #[test]
    fn never_automatically_selects_vpn() {
        let routes = [
            route(InterfaceKind::Ethernet, 100.0, 2.0),
            route(InterfaceKind::Vpn, 10_000.0, 0.1),
        ];
        assert_eq!(
            select_best_route(&routes).unwrap().interface.kind,
            InterfaceKind::Ethernet
        );
    }
}
