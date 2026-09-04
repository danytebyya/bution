//! Lifecycle and log capture for llama.cpp child processes.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessKind {
    LlamaServer,
    RpcWorker,
    LlamaBench,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessLog {
    pub process: ProcessKind,
    pub stream: OutputStream,
    pub line: String,
}

#[derive(Debug, Clone)]
pub struct ProcessSpec {
    pub kind: ProcessKind,
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub environment: Vec<(String, String)>,
}

impl ProcessSpec {
    pub fn new(kind: ProcessKind, executable: impl Into<PathBuf>) -> Self {
        Self {
            kind,
            executable: executable.into(),
            args: Vec::new(),
            working_directory: None,
            environment: Vec::new(),
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
}

struct ManagedProcess {
    child: Child,
    output_tasks: Vec<JoinHandle<()>>,
}

pub struct ProcessManager {
    processes: HashMap<ProcessKind, ManagedProcess>,
    logs: broadcast::Sender<ProcessLog>,
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new(1_024)
    }
}

impl ProcessManager {
    pub fn new(log_capacity: usize) -> Self {
        let (logs, _) = broadcast::channel(log_capacity.max(16));
        Self {
            processes: HashMap::new(),
            logs,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProcessLog> {
        self.logs.subscribe()
    }

    pub async fn start(&mut self, spec: ProcessSpec) -> Result<()> {
        if self.processes.contains_key(&spec.kind) {
            bail!("a {:?} process is already managed", spec.kind);
        }
        let mut command = Command::new(&spec.executable);
        command
            .args(&spec.args)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(directory) = &spec.working_directory {
            command.current_dir(directory);
        }
        command.envs(spec.environment.iter().map(|(key, value)| (key, value)));
        let mut child = command.spawn().with_context(|| {
            format!(
                "could not start {:?} using {}",
                spec.kind,
                spec.executable.display()
            )
        })?;

        let mut output_tasks = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            output_tasks.push(capture_lines(
                stdout,
                spec.kind,
                OutputStream::Stdout,
                self.logs.clone(),
            ));
        }
        if let Some(stderr) = child.stderr.take() {
            output_tasks.push(capture_lines(
                stderr,
                spec.kind,
                OutputStream::Stderr,
                self.logs.clone(),
            ));
        }
        self.processes.insert(
            spec.kind,
            ManagedProcess {
                child,
                output_tasks,
            },
        );
        Ok(())
    }

    pub fn is_running(&mut self, kind: ProcessKind) -> bool {
        self.processes
            .get_mut(&kind)
            .is_some_and(|process| matches!(process.child.try_wait(), Ok(None)))
    }

    pub async fn wait_for_exit(
        &mut self,
        kind: ProcessKind,
        timeout: Duration,
    ) -> Result<ExitStatus> {
        let process = self
            .processes
            .get_mut(&kind)
            .with_context(|| format!("no {kind:?} process is managed"))?;
        let status = tokio::time::timeout(timeout, process.child.wait())
            .await
            .context("child process did not exit in time")??;
        if let Some(process) = self.processes.remove(&kind) {
            for task in process.output_tasks {
                let _ = task.await;
            }
        }
        Ok(status)
    }

    pub async fn stop(&mut self, kind: ProcessKind) -> Result<()> {
        let Some(mut process) = self.processes.remove(&kind) else {
            return Ok(());
        };
        if process.child.try_wait()?.is_none() {
            process
                .child
                .start_kill()
                .with_context(|| format!("could not stop {kind:?}"))?;
            let _ = tokio::time::timeout(Duration::from_secs(5), process.child.wait()).await;
        }
        for task in process.output_tasks {
            let _ = task.await;
        }
        Ok(())
    }

    pub async fn stop_all(&mut self) {
        let kinds: Vec<_> = self.processes.keys().copied().collect();
        for kind in kinds {
            let _ = self.stop(kind).await;
        }
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        for process in self.processes.values_mut() {
            let _ = process.child.start_kill();
            for task in &process.output_tasks {
                task.abort();
            }
        }
    }
}

fn capture_lines<R>(
    reader: R,
    process: ProcessKind,
    stream: OutputStream,
    logs: broadcast::Sender<ProcessLog>,
) -> JoinHandle<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = logs.send(ProcessLog {
                process,
                stream,
                line,
            });
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_output_and_reaps_child() {
        let mut manager = ProcessManager::default();
        let mut logs = manager.subscribe();
        manager
            .start(ProcessSpec::new(ProcessKind::LlamaBench, "rustc").args(["--version"]))
            .await
            .unwrap();
        let status = manager
            .wait_for_exit(ProcessKind::LlamaBench, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(status.success());
        let log = logs.try_recv().unwrap();
        assert!(log.line.starts_with("rustc "));
        assert!(!manager.is_running(ProcessKind::LlamaBench));
    }
}
