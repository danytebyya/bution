//! Streaming terminal chat client for the local llama-server endpoint.

use crate::locale::text;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    Token(String),
    Finished { tokens_per_second: Option<f64> },
    Error { message: String, detail: String },
}

#[derive(Clone)]
pub struct ChatClient {
    client: Client,
    endpoint: String,
}

impl ChatClient {
    pub fn local(server: SocketAddr) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .expect("valid HTTP client"),
            endpoint: format!("http://{server}/v1/chat/completions"),
        }
    }

    pub fn stream_completion(
        &self,
        messages: Vec<ChatMessage>,
        temperature: f32,
    ) -> mpsc::Receiver<ChatEvent> {
        let (sender, receiver) = mpsc::channel(256);
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        tokio::spawn(async move {
            let response = client
                .post(endpoint)
                .json(&json!({
                    "messages": messages,
                    "temperature": temperature,
                    "stream": true
                }))
                .send()
                .await;
            let response = match response {
                Ok(response) if response.status().is_success() => response,
                Ok(response) => {
                    let status = response.status();
                    let detail = response.text().await.unwrap_or_default();
                    let _ = sender
                        .send(ChatEvent::Error {
                            message: text(
                                "The model server could not answer",
                                "Сервер модели не смог ответить",
                            )
                            .into(),
                            detail: format!("HTTP {status}: {detail}"),
                        })
                        .await;
                    return;
                }
                Err(error) => {
                    let _ = sender
                        .send(ChatEvent::Error {
                            message: text(
                                "The local model server is unavailable",
                                "Локальный сервер модели недоступен",
                            )
                            .into(),
                            detail: error.to_string(),
                        })
                        .await;
                    return;
                }
            };

            let mut decoder = SseDecoder::default();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(chunk) => {
                        for data in decoder.push(&chunk) {
                            if handle_sse_data(&sender, &data).await {
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = sender
                            .send(ChatEvent::Error {
                                message: text(
                                    "The streaming response was interrupted",
                                    "Получение ответа прервано",
                                )
                                .into(),
                                detail: error.to_string(),
                            })
                            .await;
                        return;
                    }
                }
            }
            let _ = sender
                .send(ChatEvent::Finished {
                    tokens_per_second: None,
                })
                .await;
        });
        receiver
    }
}

async fn handle_sse_data(sender: &mpsc::Sender<ChatEvent>, data: &str) -> bool {
    if data == "[DONE]" {
        let _ = sender
            .send(ChatEvent::Finished {
                tokens_per_second: None,
            })
            .await;
        return true;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return false;
    };
    if let Some(token) = value
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
    {
        let _ = sender.send(ChatEvent::Token(token.into())).await;
    }
    if let Some(speed) = value
        .pointer("/timings/predicted_per_second")
        .and_then(Value::as_f64)
    {
        let _ = sender
            .send(ChatEvent::Finished {
                tokens_per_second: Some(speed),
            })
            .await;
        return true;
    }
    false
}

#[derive(Default)]
struct SseDecoder {
    buffer: String,
}

impl SseDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        self.buffer = self.buffer.replace("\r\n", "\n");
        let mut events = Vec::new();
        while let Some(boundary) = self.buffer.find("\n\n") {
            let block = self.buffer[..boundary].to_owned();
            self.buffer.drain(..boundary + 2);
            let data = block
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(str::trim_start)
                .collect::<Vec<_>>()
                .join("\n");
            if !data.is_empty() {
                events.push(data);
            }
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_sse_across_arbitrary_chunks() {
        let mut decoder = SseDecoder::default();
        assert!(decoder.push(b"data: {\"choices\":").is_empty());
        let events = decoder.push(b"[]}\r\n\r\ndata: [DONE]\n\n");
        assert_eq!(events, vec![r#"{"choices":[]}"#, "[DONE]"]);
    }

    #[tokio::test]
    async fn extracts_streamed_token() {
        let (sender, mut receiver) = mpsc::channel(2);
        let done = handle_sse_data(&sender, r#"{"choices":[{"delta":{"content":"hello"}}]}"#).await;
        assert!(!done);
        assert_eq!(
            receiver.recv().await,
            Some(ChatEvent::Token("hello".into()))
        );
    }
}
