//! llama.cpp binary discovery and safe command construction.

use crate::processes::{ProcessKind, ProcessSpec};
use anyhow::{Context, Result, bail};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlamaBinaries {
    pub server: PathBuf,
    pub rpc_server: PathBuf,
    pub bench: PathBuf,
}

impl LlamaBinaries {
    pub fn discover(bin_directory: Option<&Path>) -> Result<Self> {
        Ok(Self {
            server: find_binary(bin_directory, &["llama-server"])
                .context("llama-server is not installed")?,
            rpc_server: find_binary(bin_directory, &["rpc-server", "ggml-rpc-server"])
                .context("llama.cpp RPC server is not installed")?,
            bench: find_binary(bin_directory, &["llama-bench"])
                .context("llama-bench is not installed")?,
        })
    }

    pub fn worker_process(&self, config: &WorkerConfig) -> Result<ProcessSpec> {
        if config.bind_address.is_unspecified() || config.bind_address.is_loopback() {
            bail!("RPC worker must bind to the selected trusted LAN address");
        }
        let mut args = vec![
            "--host".into(),
            config.bind_address.to_string(),
            "--port".into(),
            config.port.to_string(),
            "--threads".into(),
            config.threads.max(1).to_string(),
        ];
        if config.enable_cache {
            args.push("--cache".into());
        }
        Ok(ProcessSpec::new(ProcessKind::RpcWorker, &self.rpc_server).args(args))
    }

    pub fn server_process(&self, config: &ServerConfig) -> Result<ProcessSpec> {
        validate_model(&config.model)?;
        let mut args = vec![
            "--model".into(),
            config.model.display().to_string(),
            "--host".into(),
            config.bind_address.to_string(),
            "--port".into(),
            config.port.to_string(),
            "--ctx-size".into(),
            config.context_size.to_string(),
            "--n-gpu-layers".into(),
            "all".into(),
        ];
        if !config.rpc_endpoints.is_empty() {
            args.extend([
                "--rpc".into(),
                join_endpoints(&config.rpc_endpoints),
                "--split-mode".into(),
                "layer".into(),
            ]);
        }
        if !config.tensor_split.is_empty() {
            validate_split(&config.tensor_split)?;
            args.extend([
                "--tensor-split".into(),
                join_split(&config.tensor_split, ","),
            ]);
        }
        Ok(ProcessSpec::new(ProcessKind::LlamaServer, &self.server).args(args))
    }

    pub fn bench_process(&self, config: &BenchConfig) -> Result<ProcessSpec> {
        validate_model(&config.model)?;
        let mut args = vec![
            "--model".into(),
            config.model.display().to_string(),
            "--output".into(),
            "json".into(),
            "--n-prompt".into(),
            config.prompt_tokens.to_string(),
            "--n-gen".into(),
            config.generation_tokens.to_string(),
            "--repetitions".into(),
            config.repetitions.max(1).to_string(),
            "--n-gpu-layers".into(),
            "all".into(),
        ];
        if !config.rpc_endpoints.is_empty() {
            args.extend(["--rpc".into(), join_endpoints(&config.rpc_endpoints)]);
        }
        if !config.tensor_split.is_empty() {
            validate_split(&config.tensor_split)?;
            args.extend([
                "--tensor-split".into(),
                join_split(&config.tensor_split, "/"),
            ]);
        }
        Ok(ProcessSpec::new(ProcessKind::LlamaBench, &self.bench).args(args))
    }
}

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub bind_address: IpAddr,
    pub port: u16,
    pub threads: usize,
    pub enable_cache: bool,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub model: PathBuf,
    pub bind_address: IpAddr,
    pub port: u16,
    pub context_size: usize,
    pub rpc_endpoints: Vec<SocketAddr>,
    pub tensor_split: Vec<f32>,
}

impl ServerConfig {
    pub fn local(model: PathBuf) -> Self {
        Self {
            model,
            bind_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 8_080,
            context_size: 4_096,
            rpc_endpoints: Vec::new(),
            tensor_split: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BenchConfig {
    pub model: PathBuf,
    pub rpc_endpoints: Vec<SocketAddr>,
    pub tensor_split: Vec<f32>,
    pub prompt_tokens: usize,
    pub generation_tokens: usize,
    pub repetitions: usize,
}

fn validate_model(model: &Path) -> Result<()> {
    if model.extension().and_then(|extension| extension.to_str()) != Some("gguf") {
        bail!("the selected model must be a .gguf file");
    }
    if !model.is_file() {
        bail!(
            "the selected GGUF model does not exist: {}",
            model.display()
        );
    }
    Ok(())
}

fn validate_split(split: &[f32]) -> Result<()> {
    if split
        .iter()
        .any(|weight| !weight.is_finite() || *weight <= 0.0)
    {
        bail!("tensor split weights must be finite positive numbers");
    }
    Ok(())
}

fn join_endpoints(endpoints: &[SocketAddr]) -> String {
    endpoints
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn join_split(split: &[f32], separator: &str) -> String {
    split
        .iter()
        .map(|weight| format!("{weight:.3}"))
        .collect::<Vec<_>>()
        .join(separator)
}

fn find_binary(directory: Option<&Path>, candidates: &[&str]) -> Option<PathBuf> {
    if let Some(directory) = directory {
        for candidate in candidates {
            let path = executable_path(directory.join(candidate));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for candidate in candidates {
            let path = executable_path(directory.join(candidate));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn executable_path(path: PathBuf) -> PathBuf {
    if cfg!(windows) && path.extension().is_none() {
        path.with_extension("exe")
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fake_binaries() -> (PathBuf, LlamaBinaries, PathBuf) {
        let directory = std::env::temp_dir().join(format!("bution-bin-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let binary = directory.join("fake");
        let model = directory.join("model.gguf");
        fs::write(&binary, b"").unwrap();
        fs::write(&model, b"GGUF").unwrap();
        (
            directory,
            LlamaBinaries {
                server: binary.clone(),
                rpc_server: binary.clone(),
                bench: binary,
            },
            model,
        )
    }

    #[test]
    fn builds_rpc_server_without_shell() {
        let (directory, binaries, _) = fake_binaries();
        let spec = binaries
            .worker_process(&WorkerConfig {
                bind_address: "192.168.1.18".parse().unwrap(),
                port: 50_052,
                threads: 6,
                enable_cache: true,
            })
            .unwrap();
        assert_eq!(
            spec.args,
            [
                "--host",
                "192.168.1.18",
                "--port",
                "50052",
                "--threads",
                "6",
                "--cache"
            ]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn builds_distributed_llama_server_command() {
        let (directory, binaries, model) = fake_binaries();
        let mut config = ServerConfig::local(model);
        config.rpc_endpoints = vec!["192.168.1.18:50052".parse().unwrap()];
        config.tensor_split = vec![0.7, 0.3];
        let spec = binaries.server_process(&config).unwrap();
        assert!(
            spec.args
                .windows(2)
                .any(|pair| pair == ["--rpc", "192.168.1.18:50052"])
        );
        assert!(
            spec.args
                .windows(2)
                .any(|pair| pair == ["--tensor-split", "0.700,0.300"])
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_non_gguf_model() {
        let (directory, binaries, _) = fake_binaries();
        let config = ServerConfig::local(directory.join("model.bin"));
        assert!(binaries.server_process(&config).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
