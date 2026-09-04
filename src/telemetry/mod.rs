//! One-second runtime telemetry for the local node.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use sysinfo::{Networks, System};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetrySample {
    pub captured_at: DateTime<Utc>,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub cpu_percent: f32,
    pub gpu_percent: Option<f32>,
    pub network_receive_mbps: f64,
    pub network_send_mbps: f64,
    pub generation_tokens_per_second: Option<f64>,
}

pub struct TelemetryCollector {
    system: System,
    networks: Networks,
    last_sample: Instant,
    generation_tokens_per_second: Option<f64>,
}

impl Default for TelemetryCollector {
    fn default() -> Self {
        let mut system = System::new();
        system.refresh_memory();
        system.refresh_cpu_all();
        let networks = Networks::new_with_refreshed_list();
        Self {
            system,
            networks,
            last_sample: Instant::now(),
            generation_tokens_per_second: None,
        }
    }
}

impl TelemetryCollector {
    pub fn set_generation_speed(&mut self, speed: Option<f64>) {
        self.generation_tokens_per_second = speed;
    }

    pub fn sample(&mut self) -> TelemetrySample {
        let elapsed = self.last_sample.elapsed().as_secs_f64().max(0.001);
        self.system.refresh_memory();
        self.system.refresh_cpu_usage();
        self.networks.refresh(true);
        let received: u64 = self
            .networks
            .values()
            .map(|network| network.received())
            .sum();
        let sent: u64 = self
            .networks
            .values()
            .map(|network| network.transmitted())
            .sum();
        self.last_sample = Instant::now();
        TelemetrySample {
            captured_at: Utc::now(),
            memory_used_bytes: self.system.used_memory(),
            memory_total_bytes: self.system.total_memory(),
            cpu_percent: self.system.global_cpu_usage(),
            // Portable GPU utilization requires backend-specific APIs. Hardware
            // capability is still reported; None is rendered as unavailable.
            gpu_percent: None,
            network_receive_mbps: received as f64 * 8.0 / elapsed / 1_000_000.0,
            network_send_mbps: sent as f64 * 8.0 / elapsed / 1_000_000.0,
            generation_tokens_per_second: self.generation_tokens_per_second,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_finite_local_telemetry() {
        let mut collector = TelemetryCollector::default();
        collector.set_generation_speed(Some(8.2));
        let sample = collector.sample();
        assert!(sample.memory_total_bytes > 0);
        assert!(sample.cpu_percent.is_finite());
        assert_eq!(sample.generation_tokens_per_second, Some(8.2));
    }
}
