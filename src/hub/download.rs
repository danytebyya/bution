//! Resumable streaming downloads into the standard BUTION models directory.

use crate::hub::{
    HubEvent,
    huggingface::{HubFile, HuggingFaceClient},
};
use crate::models::ModelInfo;
use anyhow::{Context, Result, bail};
use futures_util::StreamExt;
use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, RANGE},
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, watch};

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub filename: String,
    pub destination: PathBuf,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: f64,
}

pub enum DownloadOutcome {
    Finished(ModelInfo),
    Cancelled(PathBuf),
}

pub async fn download_file(
    client: &HuggingFaceClient,
    remote: &HubFile,
    models_dir: &Path,
    mut cancelled: watch::Receiver<bool>,
    events: &mpsc::Sender<HubEvent>,
) -> Result<DownloadOutcome> {
    tokio::fs::create_dir_all(models_dir).await?;
    let basename = Path::new(&remote.filename)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .context("Hub file has no safe local filename")?;
    let destination = models_dir.join(basename);
    if destination.exists() {
        return Ok(DownloadOutcome::Finished(ModelInfo::inspect(destination)?));
    }
    let part = destination.with_extension("gguf.part");
    let mut existing = tokio::fs::metadata(&part)
        .await
        .map(|value| value.len())
        .unwrap_or(0);
    if existing > remote.size_bytes {
        tokio::fs::File::create(&part).await?;
        existing = 0;
    }
    let remaining = remote.size_bytes.saturating_sub(existing);
    let free = fs2::available_space(models_dir)
        .with_context(|| format!("could not check free space in {}", models_dir.display()))?;
    if free < remaining {
        bail!(
            "not enough free space: need {:.1} GiB, have {:.1} GiB",
            remaining as f64 / 1024_f64.powi(3),
            free as f64 / 1024_f64.powi(3)
        );
    }
    if existing == remote.size_bytes {
        return finalize_download(&part, &destination).await;
    }

    let url = client.download_url(remote)?;
    let mut request = client.http().get(url.clone());
    if existing > 0 {
        request = request.header(RANGE, format!("bytes={existing}-"));
    }
    let response = request.send().await?;
    let resumed = existing > 0
        && response.status() == StatusCode::PARTIAL_CONTENT
        && response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with(&format!("bytes {existing}-")));
    let response = if existing > 0 && !resumed {
        existing = 0;
        client.http().get(url).send().await?
    } else {
        response
    }
    .error_for_status()?;
    let mut output = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .await
        .with_context(|| format!("could not open {}", part.display()))?;
    let started = Instant::now();
    let mut last_update = Instant::now() - Duration::from_secs(1);
    let mut downloaded = existing;
    let mut stream = response.bytes_stream();
    loop {
        tokio::select! {
            changed = cancelled.changed() => {
                if changed.is_ok() && *cancelled.borrow() {
                    output.flush().await?;
                    return Ok(DownloadOutcome::Cancelled(part));
                }
            }
            chunk = stream.next() => match chunk {
                Some(chunk) => {
                    let chunk = chunk?;
                    output.write_all(&chunk).await?;
                    downloaded = downloaded.saturating_add(chunk.len() as u64);
                    if last_update.elapsed() >= Duration::from_millis(200) || downloaded == remote.size_bytes {
                        let elapsed = started.elapsed().as_secs_f64().max(0.001);
                        let speed = downloaded.saturating_sub(existing) as f64 / elapsed;
                        let _ = events.send(HubEvent::DownloadProgress(DownloadProgress {
                            filename: basename.to_owned(),
                            destination: destination.clone(),
                            downloaded_bytes: downloaded,
                            total_bytes: remote.size_bytes,
                            bytes_per_second: speed,
                        })).await;
                        last_update = Instant::now();
                    }
                }
                None => break,
            }
        }
    }
    output.flush().await?;
    output.sync_all().await?;
    drop(output);
    let actual = tokio::fs::metadata(&part).await?.len();
    if actual != remote.size_bytes {
        bail!(
            "download is incomplete: expected {} bytes, received {}",
            remote.size_bytes,
            actual
        );
    }
    finalize_download(&part, &destination).await
}

async fn finalize_download(part: &Path, destination: &Path) -> Result<DownloadOutcome> {
    tokio::fs::rename(part, destination).await?;
    match ModelInfo::inspect(destination) {
        Ok(model) => Ok(DownloadOutcome::Finished(model)),
        Err(error) => {
            let _ = tokio::fs::rename(destination, part).await;
            Err(error.context("downloaded file did not pass GGUF validation"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::quantization::Quantization;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn resumes_part_file_and_atomically_finishes_valid_gguf() {
        let mut body = b"GGUF\x03\x00\x00\x00".to_vec();
        body.extend((0..8192).map(|index| (index % 251) as u8));
        let resume_at = 777_usize;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let served = body.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0_u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains(&format!("range: bytes={resume_at}-"))
            );
            let remainder = &served[resume_at..];
            let headers = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                remainder.len(),
                resume_at,
                served.len() - 1,
                served.len()
            );
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(remainder).await.unwrap();
        });

        let directory =
            std::env::temp_dir().join(format!("bution-download-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("model-Q4_K_M.gguf");
        let part = destination.with_extension("gguf.part");
        std::fs::write(&part, &body[..resume_at]).unwrap();
        let client = HuggingFaceClient::with_endpoint(
            reqwest::Url::parse(&format!("http://{address}/")).unwrap(),
        )
        .unwrap();
        let remote = HubFile {
            repository: "owner/model".into(),
            revision: "commit".into(),
            filename: "model-Q4_K_M.gguf".into(),
            size_bytes: body.len() as u64,
            quantization: Quantization("Q4_K_M".into()),
        };
        let (_cancel, receiver) = watch::channel(false);
        let (sender, _events) = mpsc::channel(8);
        let outcome = download_file(&client, &remote, &directory, receiver, &sender)
            .await
            .unwrap();
        assert!(matches!(outcome, DownloadOutcome::Finished(_)));
        assert_eq!(std::fs::read(&destination).unwrap(), body);
        assert!(!part.exists());
        server.await.unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
