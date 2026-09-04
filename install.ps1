$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Write-Step([string]$Message) {
    Write-Host "BUTION $Message" -ForegroundColor Cyan
}

if ($env:PROCESSOR_ARCHITECTURE -notin @("AMD64", "x86_64")) {
    throw "Этот установщик пока поддерживает Windows x64."
}

$Repo = "danytebyya/bution"
$InstallDir = Join-Path $env:LOCALAPPDATA "BUTION"
$BinDir = Join-Path $InstallDir "bin"
$LlamaDir = Join-Path $InstallDir "llama"
$TemporaryDir = Join-Path ([IO.Path]::GetTempPath()) ("bution-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $BinDir, $LlamaDir, $TemporaryDir | Out-Null

try {
    Write-Step "загружаю BUTION…"
    $ButionArchive = Join-Path $TemporaryDir "bution.zip"
    $ButionUrl = "https://github.com/$Repo/releases/latest/download/bution-windows-x64.zip"
    Invoke-WebRequest -UseBasicParsing -Uri $ButionUrl -OutFile $ButionArchive
    Expand-Archive -Force -Path $ButionArchive -DestinationPath $TemporaryDir
    Copy-Item -Force (Join-Path $TemporaryDir "bution.exe") (Join-Path $BinDir "bution-real.exe")

    Write-Step "загружаю официальный llama.cpp с RPC…"
    $Release = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest"
    $Asset = $Release.assets |
        Where-Object { $_.name -match "bin-win-cpu-x64\.zip$" } |
        Select-Object -First 1
    if (-not $Asset) {
        throw "GitHub не вернул архив llama.cpp для Windows x64."
    }
    $LlamaArchive = Join-Path $TemporaryDir "llama.zip"
    $LlamaExtract = Join-Path $TemporaryDir "llama"
    Invoke-WebRequest -UseBasicParsing -Uri $Asset.browser_download_url -OutFile $LlamaArchive
    Expand-Archive -Force -Path $LlamaArchive -DestinationPath $LlamaExtract
    $LlamaServer = Get-ChildItem -Path $LlamaExtract -Recurse -File -Filter "llama-server.exe" |
        Select-Object -First 1
    if (-not $LlamaServer) {
        throw "В архиве llama.cpp отсутствует llama-server.exe."
    }
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $LlamaDir "*")
    Copy-Item -Recurse -Force (Join-Path $LlamaServer.Directory.FullName "*") $LlamaDir
    if (-not (Test-Path (Join-Path $LlamaDir "llama-bench.exe"))) {
        throw "В архиве llama.cpp отсутствует llama-bench.exe."
    }
    if (-not ((Test-Path (Join-Path $LlamaDir "rpc-server.exe")) -or
              (Test-Path (Join-Path $LlamaDir "ggml-rpc-server.exe")))) {
        throw "В архиве llama.cpp отсутствует RPC server."
    }

    $Launcher = Join-Path $BinDir "bution.cmd"
    @"
@echo off
"$BinDir\bution-real.exe" --llama-bin-dir "$LlamaDir" %*
"@ | Set-Content -Encoding ASCII $Launcher

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (($UserPath -split ";") -notcontains $BinDir) {
        $NewPath = if ([string]::IsNullOrWhiteSpace($UserPath)) { $BinDir } else { "$UserPath;$BinDir" }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    }
    $env:Path = "$BinDir;$env:Path"

    Write-Step "настраиваю Windows Firewall для частной сети…"
    $FirewallScript = @'
$ErrorActionPreference = "Stop"
$tcp = Get-NetFirewallRule -DisplayName "BUTION TCP" -ErrorAction SilentlyContinue
if (-not $tcp) {
    New-NetFirewallRule -DisplayName "BUTION TCP" -Direction Inbound -Protocol TCP -LocalPort 31750,31751,50052 -Action Allow -Profile Private | Out-Null
}
$udp = Get-NetFirewallRule -DisplayName "BUTION mDNS" -ErrorAction SilentlyContinue
if (-not $udp) {
    New-NetFirewallRule -DisplayName "BUTION mDNS" -Direction Inbound -Protocol UDP -LocalPort 5353 -Action Allow -Profile Private | Out-Null
}
'@
    $Encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($FirewallScript))
    Start-Process powershell.exe -Verb RunAs -Wait -ArgumentList "-NoProfile", "-EncodedCommand", $Encoded

    Write-Host ""
    Write-Host "Установка завершена." -ForegroundColor Green
    Write-Host "Запустить сейчас:"
    Write-Host "  $Launcher"
    Write-Host ""
    Write-Host "В новом PowerShell можно использовать просто:"
    Write-Host "  bution"
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TemporaryDir
}

