//! Long-lived Ed25519 node identity and pairing verification.

mod noise;

pub use noise::{NoiseChannel, NoiseIdentity};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use uuid::Uuid;

pub struct NodeIdentity {
    signing_key: SigningKey,
}

impl NodeIdentity {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let encoded = fs::read_to_string(path)
                .with_context(|| format!("could not read identity from {}", path.display()))?;
            let bytes = STANDARD
                .decode(encoded.trim())
                .context("identity key is not valid base64")?;
            let key_bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("identity key must contain exactly 32 bytes"))?;
            return Ok(Self {
                signing_key: SigningKey::from_bytes(&key_bytes),
            });
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("could not create identity directory")?;
        }
        let identity = Self {
            signing_key: SigningKey::generate(&mut OsRng),
        };
        fs::write(path, STANDARD.encode(identity.signing_key.to_bytes()))
            .with_context(|| format!("could not save identity to {}", path.display()))?;
        restrict_key_permissions(path)?;
        Ok(identity)
    }

    pub fn public_key(&self) -> String {
        STANDARD.encode(self.signing_key.verifying_key().to_bytes())
    }

    pub fn sign(&self, payload: &[u8]) -> String {
        STANDARD.encode(self.signing_key.sign(payload).to_bytes())
    }

    pub fn verify(public_key: &str, payload: &[u8], signature: &str) -> Result<()> {
        let public_bytes = STANDARD
            .decode(public_key)
            .context("peer public key is not valid base64")?;
        let public_bytes: [u8; 32] = public_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("peer public key must contain exactly 32 bytes"))?;
        let verifying_key =
            VerifyingKey::from_bytes(&public_bytes).context("peer public key is invalid")?;
        let signature_bytes = STANDARD
            .decode(signature)
            .context("signature is not valid base64")?;
        let signature = Signature::from_slice(&signature_bytes).context("signature is invalid")?;
        verifying_key
            .verify(payload, &signature)
            .context("signature verification failed")
    }
}

/// Produces the same six-digit out-of-band confirmation on both peers.
pub fn pairing_code(local_id: Uuid, local_key: &str, remote_id: Uuid, remote_key: &str) -> String {
    let mut peers = [
        format!("{local_id}:{local_key}"),
        format!("{remote_id}:{remote_key}"),
    ];
    peers.sort();
    let digest = Sha256::digest(peers.join("|").as_bytes());
    let value = u32::from_be_bytes(digest[..4].try_into().expect("SHA-256 prefix")) % 1_000_000;
    format!("{:03} {:03}", value / 1_000, value % 1_000)
}

pub fn pairing_payload(node_id: Uuid, peer_id: Uuid, decision: &str) -> Vec<u8> {
    format!("bution-pair-v1:{node_id}:{peer_id}:{decision}").into_bytes()
}

pub fn validate_distinct_identity(local_id: Uuid, remote_id: Uuid) -> Result<()> {
    if local_id == remote_id {
        bail!("a node cannot pair with itself");
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn restrict_key_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .context("could not restrict identity key permissions")
}

#[cfg(not(unix))]
pub(super) fn restrict_key_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_key_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("bution-key-{}", Uuid::new_v4()))
    }

    #[test]
    fn identity_is_persistent_and_signatures_verify() {
        let path = temporary_key_path();
        let first = NodeIdentity::load_or_create(&path).unwrap();
        let signature = first.sign(b"pair me");
        let second = NodeIdentity::load_or_create(&path).unwrap();
        assert_eq!(first.public_key(), second.public_key());
        NodeIdentity::verify(&second.public_key(), b"pair me", &signature).unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn pairing_code_is_symmetric_and_formatted() {
        let left = Uuid::new_v4();
        let right = Uuid::new_v4();
        let forward = pairing_code(left, "left-key", right, "right-key");
        let reverse = pairing_code(right, "right-key", left, "left-key");
        assert_eq!(forward, reverse);
        assert_eq!(forward.len(), 7);
        assert_eq!(forward.as_bytes()[3], b' ');
    }

    #[test]
    fn modified_payload_fails_verification() {
        let path = temporary_key_path();
        let identity = NodeIdentity::load_or_create(&path).unwrap();
        let signature = identity.sign(b"accepted");
        assert!(NodeIdentity::verify(&identity.public_key(), b"rejected", &signature).is_err());
        fs::remove_file(path).unwrap();
    }
}
