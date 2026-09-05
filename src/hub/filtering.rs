//! Selection of standalone, primary model GGUF files.

use crate::hub::quantization::Quantization;

const EXCLUDED: [&str; 5] = ["mmproj", "draft", "projector", "vision", "clip"];

pub fn primary_quantization(filename: &str) -> Option<Quantization> {
    let lower = filename.to_ascii_lowercase();
    if !lower.ends_with(".gguf") || EXCLUDED.iter().any(|word| lower.contains(word)) {
        return None;
    }
    // llama.cpp split GGUFs must be downloaded as a set, so the single-file MVP
    // does not present one shard as if it were a runnable model.
    if lower.contains("-of-") {
        return None;
    }
    Quantization::parse(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_auxiliary_and_split_ggufs() {
        assert!(primary_quantization("Qwen-Q4_K_M.gguf").is_some());
        for file in [
            "mmproj-Q5_K_M.gguf",
            "vision-Q8_0.gguf",
            "draft-Q4.gguf",
            "model-Q4_K_M-00001-of-00003.gguf",
            "model.safetensors",
        ] {
            assert!(primary_quantization(file).is_none(), "{file}");
        }
    }
}
