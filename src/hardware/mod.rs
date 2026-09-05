//! Cross-platform hardware inventory and safe memory budgeting.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use sysinfo::System;

pub const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeBackend {
    Metal,
    Cuda,
    Vulkan,
    Cpu,
}

impl std::fmt::Display for ComputeBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Metal => "Metal",
            Self::Cuda => "CUDA",
            Self::Vulkan => "Vulkan",
            Self::Cpu => "CPU",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareProfile {
    pub os: String,
    pub os_version: String,
    pub architecture: String,
    pub cpu: String,
    pub physical_cores: usize,
    pub logical_cores: usize,
    pub total_memory_bytes: u64,
    pub currently_available_bytes: u64,
    pub ai_memory_bytes: u64,
    pub backend: ComputeBackend,
    pub unified_memory: bool,
}

impl HardwareProfile {
    pub fn detect() -> Self {
        let mut system = System::new_all();
        system.refresh_all();

        let architecture = std::env::consts::ARCH.to_owned();
        let os = System::name().unwrap_or_else(|| std::env::consts::OS.to_owned());
        let is_macos = os.eq_ignore_ascii_case("macos")
            || os.eq_ignore_ascii_case("darwin")
            || cfg!(target_os = "macos");
        let unified_memory =
            is_macos && (architecture == "aarch64" || cfg!(target_arch = "aarch64"));
        let backend = detect_backend(unified_memory);
        let total_memory_bytes = system.total_memory();
        let currently_available_bytes = system.available_memory();

        Self {
            os,
            os_version: System::long_os_version().unwrap_or_else(|| "Unknown".into()),
            architecture,
            cpu: system
                .cpus()
                .first()
                .map(|cpu| cpu.brand().trim().to_owned())
                .filter(|brand| !brand.is_empty())
                .unwrap_or_else(|| "Unknown CPU".into()),
            physical_cores: system.physical_core_count().unwrap_or(system.cpus().len()),
            logical_cores: system.cpus().len(),
            total_memory_bytes,
            currently_available_bytes,
            ai_memory_bytes: available_for_ai(
                total_memory_bytes,
                currently_available_bytes,
                unified_memory,
            ),
            backend,
            unified_memory,
        }
    }

    pub fn total_memory_gib(&self) -> f64 {
        bytes_to_gib(self.total_memory_bytes)
    }

    pub fn ai_memory_gib(&self) -> f64 {
        bytes_to_gib(self.ai_memory_bytes)
    }
}

/// Keeps the larger of 2 GiB or 15% of physical memory for the OS.
/// On macOS Unified Memory, inactive cache is purged on Metal allocation,
/// allowing the safe budget to use physical memory minus the OS reserve.
pub fn available_for_ai(total: u64, currently_available: u64, unified_memory: bool) -> u64 {
    let reserve = (total.saturating_mul(15) / 100).max(2 * GIB);
    let safe_budget = total.saturating_sub(reserve);
    if unified_memory {
        safe_budget
    } else if currently_available > 0 {
        currently_available.min(safe_budget)
    } else {
        safe_budget
    }
}

pub fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / GIB as f64
}

fn detect_backend(unified_memory: bool) -> ComputeBackend {
    if unified_memory {
        ComputeBackend::Metal
    } else if command_exists("nvidia-smi") {
        ComputeBackend::Cuda
    } else if command_exists("vulkaninfo") {
        ComputeBackend::Vulkan
    } else {
        ComputeBackend::Cpu
    }
}

fn command_exists(command: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| {
            executable_candidates(&directory, command)
                .into_iter()
                .any(|candidate| candidate.is_file())
        })
    })
}

fn executable_candidates(directory: &std::path::Path, command: &str) -> Vec<PathBuf> {
    let base = directory.join(command);
    if cfg!(windows) {
        vec![base.with_extension("exe"), base.with_extension("cmd"), base]
    } else {
        vec![base]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_two_gib_on_small_machine() {
        assert_eq!(available_for_ai(8 * GIB, 7 * GIB, false), 6 * GIB);
    }

    #[test]
    fn reserves_fifteen_percent_on_large_machine() {
        assert_eq!(available_for_ai(32 * GIB, 32 * GIB, false), 29_205_777_613);
    }

    #[test]
    fn current_pressure_reduces_ai_budget() {
        assert_eq!(available_for_ai(32 * GIB, 4 * GIB, false), 4 * GIB);
    }

    #[test]
    fn unified_memory_uses_full_safe_budget() {
        assert_eq!(available_for_ai(16 * GIB, 1 * GIB, true), 14_602_888_807); // 13.6 GiB
        assert_eq!(available_for_ai(8 * GIB, 500 * 1024 * 1024, true), 6 * GIB);
    }
}
