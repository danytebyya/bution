param(
    [string]$ProjectDir = ".bution\llama.cpp",
    [ValidateSet("CPU", "Vulkan", "CUDA")]
    [string]$Backend = "CPU"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path "$ProjectDir\.git")) {
    git clone --depth 1 https://github.com/ggml-org/llama.cpp.git $ProjectDir
}

$cmakeArgs = @(
    "-S", $ProjectDir,
    "-B", "$ProjectDir\build",
    "-DGGML_RPC=ON",
    "-DLLAMA_CURL=OFF"
)

if ($Backend -eq "Vulkan") {
    $cmakeArgs += "-DGGML_VULKAN=ON"
}
if ($Backend -eq "CUDA") {
    $cmakeArgs += "-DGGML_CUDA=ON"
}

cmake @cmakeArgs
cmake --build "$ProjectDir\build" --config Release --parallel

Write-Host "llama.cpp binaries: $ProjectDir\build\bin\Release"
Write-Host "Run: cargo run --release -- --llama-bin-dir $ProjectDir\build\bin\Release"

