use super::restrict_key_permissions;
use crate::cluster::ControlMessage;
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use snow::{Builder, HandshakeState, TransportState, params::NoiseParams};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const NOISE_PATTERN: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const MAX_FRAME: usize = 65_535;

#[derive(Debug, Serialize, Deserialize)]
struct StoredNoiseKey {
    private_key: String,
    public_key: String,
}

#[derive(Clone)]
pub struct NoiseIdentity {
    private_key: Vec<u8>,
    public_key: Vec<u8>,
}

impl NoiseIdentity {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.is_file() {
            let stored: StoredNoiseKey =
                serde_json::from_slice(&fs::read(path).with_context(|| {
                    format!("could not read Noise identity {}", path.display())
                })?)
                .context("Noise identity is invalid")?;
            let identity = Self {
                private_key: STANDARD
                    .decode(stored.private_key)
                    .context("Noise private key is invalid")?,
                public_key: STANDARD
                    .decode(stored.public_key)
                    .context("Noise public key is invalid")?,
            };
            identity.validate()?;
            return Ok(identity);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("could not create Noise identity directory")?;
        }
        let keypair = Builder::new(parameters()?)
            .generate_keypair()
            .context("could not generate Noise identity")?;
        let identity = Self {
            private_key: keypair.private,
            public_key: keypair.public,
        };
        let stored = StoredNoiseKey {
            private_key: STANDARD.encode(&identity.private_key),
            public_key: STANDARD.encode(&identity.public_key),
        };
        fs::write(path, serde_json::to_vec(&stored)?)
            .with_context(|| format!("could not save Noise identity {}", path.display()))?;
        restrict_key_permissions(path)?;
        Ok(identity)
    }

    pub fn public_key(&self) -> String {
        STANDARD.encode(&self.public_key)
    }

    fn validate(&self) -> Result<()> {
        if self.private_key.len() != 32 || self.public_key.len() != 32 {
            bail!("Noise identity keys must contain exactly 32 bytes");
        }
        Ok(())
    }

    fn initiator(&self) -> Result<HandshakeState> {
        Builder::new(parameters()?)
            .local_private_key(&self.private_key)
            .build_initiator()
            .context("could not initialize Noise initiator")
    }

    fn responder(&self) -> Result<HandshakeState> {
        Builder::new(parameters()?)
            .local_private_key(&self.private_key)
            .build_responder()
            .context("could not initialize Noise responder")
    }
}

pub struct NoiseChannel {
    stream: TcpStream,
    transport: TransportState,
    remote_public_key: String,
}

impl NoiseChannel {
    pub async fn connect(
        address: SocketAddr,
        identity: &NoiseIdentity,
        timeout: Duration,
    ) -> Result<Self> {
        let stream = tokio::time::timeout(timeout, TcpStream::connect(address))
            .await
            .context("control connection timed out")??;
        let mut channel = HandshakeIo::new(stream, identity.initiator()?);
        channel.write().await?;
        channel.read().await?;
        channel.write().await?;
        channel.finish()
    }

    pub async fn accept(stream: TcpStream, identity: &NoiseIdentity) -> Result<Self> {
        let mut channel = HandshakeIo::new(stream, identity.responder()?);
        channel.read().await?;
        channel.write().await?;
        channel.read().await?;
        channel.finish()
    }

    pub fn remote_public_key(&self) -> &str {
        &self.remote_public_key
    }

    pub async fn send(&mut self, message: &ControlMessage) -> Result<()> {
        let plaintext = serde_json::to_vec(message).context("could not encode control message")?;
        if plaintext.len() > MAX_FRAME - 16 {
            bail!("control message is too large");
        }
        let mut encrypted = vec![0_u8; plaintext.len() + 16];
        let count = self
            .transport
            .write_message(&plaintext, &mut encrypted)
            .context("could not encrypt control message")?;
        write_frame(&mut self.stream, &encrypted[..count]).await
    }

    pub async fn receive(&mut self) -> Result<ControlMessage> {
        let encrypted = read_frame(&mut self.stream).await?;
        let mut plaintext = vec![0_u8; encrypted.len()];
        let count = self
            .transport
            .read_message(&encrypted, &mut plaintext)
            .context("could not decrypt control message")?;
        serde_json::from_slice(&plaintext[..count]).context("peer sent an invalid control message")
    }
}

struct HandshakeIo {
    stream: TcpStream,
    state: HandshakeState,
}

impl HandshakeIo {
    fn new(stream: TcpStream, state: HandshakeState) -> Self {
        Self { stream, state }
    }

    async fn write(&mut self) -> Result<()> {
        let mut message = [0_u8; MAX_FRAME];
        let count = self
            .state
            .write_message(&[], &mut message)
            .context("Noise handshake write failed")?;
        write_frame(&mut self.stream, &message[..count]).await
    }

    async fn read(&mut self) -> Result<()> {
        let message = read_frame(&mut self.stream).await?;
        let mut payload = [0_u8; MAX_FRAME];
        self.state
            .read_message(&message, &mut payload)
            .context("Noise handshake verification failed")?;
        Ok(())
    }

    fn finish(self) -> Result<NoiseChannel> {
        if !self.state.is_handshake_finished() {
            bail!("Noise handshake did not finish");
        }
        let remote_public_key = STANDARD.encode(
            self.state
                .get_remote_static()
                .context("Noise peer did not provide a static public key")?,
        );
        let transport = self
            .state
            .into_transport_mode()
            .context("could not enter encrypted Noise transport")?;
        Ok(NoiseChannel {
            stream: self.stream,
            transport,
            remote_public_key,
        })
    }
}

async fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        bail!("invalid Noise frame length");
    }
    stream.write_u32(bytes.len() as u32).await?;
    stream.write_all(bytes).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let length = stream.read_u32().await? as usize;
    if length == 0 || length > MAX_FRAME {
        bail!("peer sent an invalid Noise frame length");
    }
    let mut bytes = vec![0_u8; length];
    stream.read_exact(&mut bytes).await?;
    Ok(bytes)
}

fn parameters() -> Result<NoiseParams> {
    NOISE_PATTERN
        .parse()
        .context("invalid built-in Noise protocol parameters")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;
    use uuid::Uuid;

    fn identity() -> (std::path::PathBuf, NoiseIdentity) {
        let path = std::env::temp_dir().join(format!("bution-noise-{}", Uuid::new_v4()));
        let identity = NoiseIdentity::load_or_create(&path).unwrap();
        (path, identity)
    }

    #[tokio::test]
    async fn exchanges_encrypted_control_messages() {
        let (server_path, server_identity) = identity();
        let (client_path, client_identity) = identity();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_key = server_identity.public_key();
        let client_key = client_identity.public_key();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut channel = NoiseChannel::accept(stream, &server_identity)
                .await
                .unwrap();
            assert_eq!(channel.remote_public_key(), client_key);
            assert_eq!(
                channel.receive().await.unwrap(),
                ControlMessage::Ping { nonce: 7 }
            );
            channel
                .send(&ControlMessage::Pong { nonce: 7 })
                .await
                .unwrap();
        });
        let mut client = NoiseChannel::connect(address, &client_identity, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(client.remote_public_key(), server_key);
        client
            .send(&ControlMessage::Ping { nonce: 7 })
            .await
            .unwrap();
        assert_eq!(
            client.receive().await.unwrap(),
            ControlMessage::Pong { nonce: 7 }
        );
        server.await.unwrap();
        fs::remove_file(server_path).unwrap();
        fs::remove_file(client_path).unwrap();
    }
}
