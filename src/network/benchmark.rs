use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::Instant;

const MAGIC: &str = "BUTION-NET-1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyStats {
    pub average_ms: f64,
    pub minimum_ms: f64,
    pub jitter_ms: f64,
    pub success_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandwidthStats {
    pub megabits_per_second: f64,
    pub transferred_bytes: u64,
    pub elapsed_seconds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stability {
    Excellent,
    Good,
    Unstable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetworkBenchmark {
    pub latency: LatencyStats,
    pub bandwidth: BandwidthStats,
    pub stability: Stability,
}

pub struct BenchmarkServer {
    local_addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl BenchmarkServer {
    pub async fn start(bind_address: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(bind_address)
            .await
            .with_context(|| format!("could not bind network benchmark to {bind_address}"))?;
        let local_addr = listener.local_addr()?;
        let (shutdown, mut shutdown_receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_receiver => break,
                    connection = listener.accept() => {
                        match connection {
                            Ok((stream, _)) => { tokio::spawn(handle_connection(stream)); }
                            Err(_) => break,
                        }
                    }
                }
            }
        });
        Ok(Self {
            local_addr,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for BenchmarkServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn handle_connection(stream: TcpStream) {
    let mut stream = BufReader::new(stream);
    let mut request = String::new();
    let read = tokio::time::timeout(Duration::from_secs(3), stream.read_line(&mut request)).await;
    if !matches!(read, Ok(Ok(1..=128))) {
        return;
    }
    if request.trim() == format!("{MAGIC} PING") {
        let _ = stream.get_mut().write_all(b"PONG\n").await;
        return;
    }
    let mut parts = request.split_whitespace();
    let download_size = if parts.next() == Some(MAGIC) && parts.next() == Some("DOWNLOAD") {
        parts.next().and_then(|value| value.parse::<usize>().ok())
    } else {
        None
    };
    if let Some(requested) = download_size {
        let size = requested.min(64 * 1024 * 1024);
        let chunk = [0x42_u8; 64 * 1024];
        let mut remaining = size;
        while remaining > 0 {
            let count = remaining.min(chunk.len());
            if stream.get_mut().write_all(&chunk[..count]).await.is_err() {
                break;
            }
            remaining -= count;
        }
    }
}

pub async fn run_network_benchmark(target: SocketAddr) -> Result<NetworkBenchmark> {
    let latency = measure_latency(target, 8).await?;
    let bandwidth = measure_bandwidth(target, 8 * 1024 * 1024).await?;
    let stability = classify_stability(&latency);
    Ok(NetworkBenchmark {
        latency,
        bandwidth,
        stability,
    })
}

pub async fn measure_latency(target: SocketAddr, samples: usize) -> Result<LatencyStats> {
    if samples == 0 {
        bail!("latency benchmark requires at least one sample");
    }
    let mut measurements = Vec::with_capacity(samples);
    for _ in 0..samples {
        if let Ok(elapsed) = ping_once(target).await {
            measurements.push(elapsed.as_secs_f64() * 1_000.0);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if measurements.is_empty() {
        bail!("the peer did not answer the latency benchmark");
    }
    let average = measurements.iter().sum::<f64>() / measurements.len() as f64;
    let variance = measurements
        .iter()
        .map(|measurement| (measurement - average).powi(2))
        .sum::<f64>()
        / measurements.len() as f64;
    Ok(LatencyStats {
        average_ms: average,
        minimum_ms: measurements.iter().copied().fold(f64::INFINITY, f64::min),
        jitter_ms: variance.sqrt(),
        success_rate: measurements.len() as f64 / samples as f64,
    })
}

async fn ping_once(target: SocketAddr) -> Result<Duration> {
    let started = Instant::now();
    let stream = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(target))
        .await
        .context("latency connection timed out")??;
    let mut stream = BufReader::new(stream);
    stream
        .get_mut()
        .write_all(format!("{MAGIC} PING\n").as_bytes())
        .await?;
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_line(&mut response))
        .await
        .context("latency response timed out")??;
    if response.trim() != "PONG" {
        bail!("peer returned an invalid latency response");
    }
    Ok(started.elapsed())
}

pub async fn measure_bandwidth(target: SocketAddr, bytes: usize) -> Result<BandwidthStats> {
    if !(64 * 1024..=64 * 1024 * 1024).contains(&bytes) {
        bail!("bandwidth sample must be between 64 KiB and 64 MiB");
    }
    let mut stream = tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(target))
        .await
        .context("bandwidth connection timed out")??;
    stream
        .write_all(format!("{MAGIC} DOWNLOAD {bytes}\n").as_bytes())
        .await?;
    let started = Instant::now();
    let mut received = 0_usize;
    let mut buffer = [0_u8; 64 * 1024];
    while received < bytes {
        let limit = (bytes - received).min(buffer.len());
        let count = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer[..limit]))
            .await
            .context("bandwidth transfer timed out")??;
        if count == 0 {
            bail!("peer closed the bandwidth transfer early");
        }
        received += count;
    }
    let elapsed = started.elapsed().as_secs_f64().max(f64::EPSILON);
    Ok(BandwidthStats {
        megabits_per_second: received as f64 * 8.0 / elapsed / 1_000_000.0,
        transferred_bytes: received as u64,
        elapsed_seconds: elapsed,
    })
}

pub fn classify_stability(latency: &LatencyStats) -> Stability {
    if latency.success_rate >= 0.99 && latency.jitter_ms <= 2.0 {
        Stability::Excellent
    } else if latency.success_rate >= 0.9 && latency.jitter_ms <= 10.0 {
        Stability::Good
    } else {
        Stability::Unstable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn measures_latency_against_local_server() {
        let server = BenchmarkServer::start("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let stats = measure_latency(server.local_addr(), 3).await.unwrap();
        assert_eq!(stats.success_rate, 1.0);
        assert!(stats.average_ms >= 0.0);
        server.stop().await;
    }

    #[tokio::test]
    async fn measures_bandwidth_against_local_server() {
        let server = BenchmarkServer::start("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let stats = measure_bandwidth(server.local_addr(), 128 * 1024)
            .await
            .unwrap();
        assert_eq!(stats.transferred_bytes, 128 * 1024);
        assert!(stats.megabits_per_second > 0.0);
        server.stop().await;
    }
}
