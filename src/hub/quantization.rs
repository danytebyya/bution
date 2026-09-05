//! Quantization names supported by llama.cpp GGUF models in BUTION.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantization(pub String);

impl Quantization {
    pub fn parse(filename: &str) -> Option<Self> {
        let upper = filename.to_ascii_uppercase();
        let stem = upper.strip_suffix(".GGUF")?;
        // Longer prefixes go first so IQ4 is never mistaken for Q4.
        const PREFIXES: [&str; 11] = [
            "BF16", "F16", "IQ2", "IQ3", "IQ4", "Q2", "Q3", "Q4", "Q5", "Q6", "Q8",
        ];
        for token in stem.split(['-', '.', ' ', '/']) {
            if PREFIXES.iter().any(|prefix| {
                token == *prefix
                    || token
                        .strip_prefix(prefix)
                        .is_some_and(|suffix| suffix.starts_with('_'))
            }) {
                return Some(Self(token.to_owned()));
            }
        }
        None
    }

    pub fn label(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_quantizations() {
        for (file, expected) in [
            ("model-IQ4_XS.gguf", "IQ4_XS"),
            ("model-Q5_K_M.gguf", "Q5_K_M"),
            ("model-Q8_0.gguf", "Q8_0"),
            ("model-BF16.gguf", "BF16"),
            ("model-F16.gguf", "F16"),
        ] {
            assert_eq!(Quantization::parse(file).unwrap().label(), expected);
        }
        assert!(Quantization::parse("model-Q1.gguf").is_none());
        assert!(Quantization::parse("model.safetensors").is_none());
    }
}
