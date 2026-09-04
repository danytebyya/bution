//! llama-bench execution and robust JSON result extraction.

use crate::llama::{BenchConfig, LlamaBinaries};
use crate::processes::ProcessManager;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlamaBenchmark {
    pub prompt_tokens_per_second: f64,
    pub generation_tokens_per_second: f64,
    pub estimated_ttft_ms: f64,
    pub compute_score: f64,
}

pub async fn run_llama_benchmark(
    manager: &mut ProcessManager,
    binaries: &LlamaBinaries,
    config: &BenchConfig,
    timeout: Duration,
) -> Result<LlamaBenchmark> {
    let process = binaries.bench_process(config)?;
    let output = manager.run_to_completion(process, timeout).await?;
    if !output.status.success() {
        bail!("llama-bench could not complete the benchmark");
    }
    parse_llama_bench_json(&output.stdout)
}

pub fn parse_llama_bench_json(output: &str) -> Result<LlamaBenchmark> {
    let json_start = output
        .find('[')
        .context("llama-bench returned no JSON array")?;
    let json_end = output
        .rfind(']')
        .context("llama-bench returned incomplete JSON")?;
    let rows: Vec<Value> = serde_json::from_str(&output[json_start..=json_end])
        .context("llama-bench returned invalid JSON")?;
    let mut prompt = Vec::new();
    let mut generation = Vec::new();
    let mut prompt_tokens = 0.0_f64;
    for row in rows {
        let throughput = number(&row, "avg_ts").unwrap_or(0.0);
        let n_prompt = number(&row, "n_prompt").unwrap_or(0.0);
        let n_gen = number(&row, "n_gen").unwrap_or(0.0);
        if n_prompt > 0.0 && n_gen == 0.0 && throughput > 0.0 {
            prompt.push(throughput);
            prompt_tokens = prompt_tokens.max(n_prompt);
        } else if n_gen > 0.0 && n_prompt == 0.0 && throughput > 0.0 {
            generation.push(throughput);
        }
    }
    let prompt_tps = average(&prompt).context("llama-bench did not report prompt processing")?;
    let generation_tps =
        average(&generation).context("llama-bench did not report text generation")?;
    let estimated_ttft_ms = (prompt_tokens / prompt_tps + 1.0 / generation_tps) * 1_000.0;
    Ok(LlamaBenchmark {
        prompt_tokens_per_second: prompt_tps,
        generation_tokens_per_second: generation_tps,
        estimated_ttft_ms,
        compute_score: generation_tps * 0.7 + prompt_tps.sqrt() * 0.3,
    })
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str()?.parse::<f64>().ok())
    })
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_llama_bench_json_schema() {
        let output = r#"backend diagnostics
[
  {"n_prompt":512,"n_gen":0,"avg_ts":800.0},
  {"n_prompt":0,"n_gen":128,"avg_ts":8.2},
  {"n_prompt":"0","n_gen":"128","avg_ts":"8.0"}
]"#;
        let result = parse_llama_bench_json(output).unwrap();
        assert_eq!(result.prompt_tokens_per_second, 800.0);
        assert_eq!(result.generation_tokens_per_second, 8.1);
        assert!(result.estimated_ttft_ms > 700.0);
    }

    #[test]
    fn rejects_result_without_both_workloads() {
        let output = r#"[{"n_prompt":512,"n_gen":0,"avg_ts":800.0}]"#;
        assert!(parse_llama_bench_json(output).is_err());
    }
}
