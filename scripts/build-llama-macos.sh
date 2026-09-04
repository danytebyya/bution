#!/usr/bin/env bash
set -euo pipefail

project_dir="${1:-.bution/llama.cpp}"

if [[ ! -d "${project_dir}/.git" ]]; then
  git clone --depth 1 https://github.com/ggml-org/llama.cpp.git "${project_dir}"
fi

cmake \
  -S "${project_dir}" \
  -B "${project_dir}/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DGGML_RPC=ON \
  -DGGML_METAL=ON \
  -DLLAMA_CURL=OFF
cmake --build "${project_dir}/build" --config Release --parallel

echo "llama.cpp binaries: ${project_dir}/build/bin"
echo "Run: cargo run --release -- --llama-bin-dir ${project_dir}/build/bin"

