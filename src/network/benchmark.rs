use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
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
    }
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
}
