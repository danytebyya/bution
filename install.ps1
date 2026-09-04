$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Write-Step([string]$Message) {
    Write-Host "BUTION $Message" -ForegroundColor Cyan
}

function ConvertTo-Utf8Text([object]$Content) {
    if ($null -eq $Content) {
        return ""
    }
    if ($Content.PSObject -and $Content.PSObject.Properties["Content"]) {
        $Content = $Content.Content
    }
    if ($null -eq $Content) {
        return ""
    }
    if ($Content -is [string]) {
        return $Content.TrimStart([char]0xFEFF).Trim()
    }
    if ($Content -is [byte[]]) {
        $text = [Text.Encoding]::UTF8.GetString([byte[]]$Content)
        return $text.TrimStart([char]0xFEFF).Trim()
    }
    if ($Content -is [Array]) {
        try {
            $bytes = [byte[]]$Content
            $text = [Text.Encoding]::UTF8.GetString($bytes)
            return $text.TrimStart([char]0xFEFF).Trim()
        }
        catch {
            # fall through
        }
    }
    if ($Content -is [byte]) {
        $text = [Text.Encoding]::UTF8.GetString([byte[]]@([byte]$Content))
        return $text.TrimStart([char]0xFEFF).Trim()
    }
    return ([string]$Content).TrimStart([char]0xFEFF).Trim()
}

$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -notin @("AMD64", "x86_64")) {
    throw "Этот установщик пока поддерживает Windows x64."
}

$Repo = "danytebyya/bution"
$InstallDir = Join-Path $env:LOCALAPPDATA "BUTION"
$BinDir = Join-Path $InstallDir "bin"
$LlamaDir = Join-Path $InstallDir "llama"
$ForceUpdate = $env:BUTION_FORCE_UPDATE -eq "1"
$TemporaryDir = Join-Path ([IO.Path]::GetTempPath()) ("bution-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $BinDir, $LlamaDir, $TemporaryDir | Out-Null

try {
    $ButionBinary = Join-Path $BinDir "bution-real.exe"
    $ButionReady = (Test-Path $ButionBinary) -and ((Get-Item $ButionBinary).Length -gt 0)
    if ((-not $ForceUpdate) -and $ButionReady) {
        Write-Step "BUTION уже установлен — пропускаю загрузку."
    }
    else {
        Write-Step "загружаю BUTION…"
        $ButionArchive = Join-Path $TemporaryDir "bution.zip"
        $ButionExtract = Join-Path $TemporaryDir "bution-extract"
        $ButionUrl = "https://github.com/$Repo/releases/latest/download/bution-windows-x64.zip"
        $ButionReady = $false
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $ButionUrl -OutFile $ButionArchive
            Expand-Archive -Force -Path $ButionArchive -DestinationPath $ButionExtract
            $ButionExe = Get-ChildItem -Path $ButionExtract -Recurse -File -Filter "bution.exe" |
                Select-Object -First 1
            if ($ButionExe) {
                Copy-Item -Force $ButionExe.FullName $ButionBinary
                $ButionReady = $true
            }
        }
        catch {
            Write-Step "готовый релиз недоступен — выполняю автоматическую сборку…"
        }

        if (-not $ButionReady) {
            $ProgFilesX86 = if (${env:ProgramFiles(x86)}) { ${env:ProgramFiles(x86)} } else { $env:ProgramFiles }
            $VsWhere = Join-Path $ProgFilesX86 "Microsoft Visual Studio\Installer\vswhere.exe"
            $CppToolsReady = $false
            if (Test-Path $VsWhere) {
                $CppInstall = & $VsWhere -latest -products "*" `
                    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
                    -property installationPath
                $CppToolsReady = ($LASTEXITCODE -eq 0) -and (-not [string]::IsNullOrWhiteSpace($CppInstall))
            }

            if ($CppToolsReady) {
                Write-Step "Microsoft C++ Build Tools уже установлены."
            }
            else {
                if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
                    throw "Для автоматической установки нужен winget (Microsoft App Installer)."
                }
                Write-Step "устанавливаю Microsoft C++ Build Tools (может занять 10–30 минут)…"
                & winget.exe install --id Microsoft.VisualStudio.2022.BuildTools --exact --silent `
                    --accept-source-agreements --accept-package-agreements --disable-interactivity `
                    --override "--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
                if ($LASTEXITCODE -ne 0) {
                    throw "Не удалось автоматически установить Microsoft C++ Build Tools."
                }
            }

            $CargoCommand = Get-Command cargo.exe -ErrorAction SilentlyContinue
            $Cargo = if ($CargoCommand -and $CargoCommand.Path) {
                $CargoCommand.Path
            }
            elseif ($CargoCommand -and $CargoCommand.Definition) {
                $CargoCommand.Definition
            }
            else {
                Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
            }
            if (Test-Path $Cargo) {
                Write-Step "Rust уже установлен."
            }
            else {
                Write-Step "устанавливаю Rust…"
                $Rustup = Join-Path $TemporaryDir "rustup-init.exe"
                Invoke-WebRequest -UseBasicParsing `
                    -Uri "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe" `
                    -OutFile $Rustup
                & $Rustup -y --profile minimal --default-toolchain stable
                if ($LASTEXITCODE -ne 0) {
                    throw "Не удалось автоматически установить Rust."
                }
            }
            $CargoDir = Split-Path $Cargo
            if ($CargoDir) {
                $env:Path = "$CargoDir;$env:Path"
            }

            Write-Step "скачиваю исходники и собираю BUTION…"
            $SourceArchive = Join-Path $TemporaryDir "source.zip"
            $SourceExtract = Join-Path $TemporaryDir "source"
            Invoke-WebRequest -UseBasicParsing `
                -Uri "https://github.com/$Repo/archive/refs/heads/main.zip" `
                -OutFile $SourceArchive
            Expand-Archive -Force -Path $SourceArchive -DestinationPath $SourceExtract
            $Manifest = Get-ChildItem -Path $SourceExtract -Recurse -File -Filter "Cargo.toml" |
                Sort-Object { $_.FullName.Length } |
                Select-Object -First 1
            if (-not $Manifest) {
                throw "В архиве BUTION отсутствует Cargo.toml."
            }
            & $Cargo build --release --locked --manifest-path $Manifest.FullName
            if ($LASTEXITCODE -ne 0) {
                throw "Автоматическая сборка BUTION завершилась ошибкой."
            }
            $BuiltBinary = Join-Path $Manifest.Directory.FullName "target\release\bution.exe"
            Copy-Item -Force $BuiltBinary $ButionBinary
        }
    }

    $LlamaReady = (Test-Path (Join-Path $LlamaDir "llama-server.exe")) -and
        (Test-Path (Join-Path $LlamaDir "llama-bench.exe")) -and
        ((Test-Path (Join-Path $LlamaDir "rpc-server.exe")) -or
         (Test-Path (Join-Path $LlamaDir "ggml-rpc-server.exe")))
    if ((-not $ForceUpdate) -and $LlamaReady) {
        Write-Step "llama.cpp с RPC уже установлен — пропускаю загрузку."
    }
    else {
        Write-Step "загружаю официальный llama.cpp с RPC…"
        $LlamaTagUrl = "https://github.com/ggml-org/llama.cpp/releases/download/v0.3.0/nightly-tag.txt"
        $LlamaTagResponse = Invoke-WebRequest -UseBasicParsing -Uri $LlamaTagUrl
        $LlamaTag = ConvertTo-Utf8Text $LlamaTagResponse.Content
        if ($LlamaTag -notmatch "^b[0-9]+$") {
            throw "Не удалось определить актуальную сборку llama.cpp: '$LlamaTag'."
        }
        $LlamaAsset = "llama-$LlamaTag-bin-win-cpu-x64.zip"
        $LlamaUrl = "https://github.com/ggml-org/llama.cpp/releases/download/$LlamaTag/$LlamaAsset"
        $LlamaArchive = Join-Path $TemporaryDir "llama.zip"
        $LlamaExtract = Join-Path $TemporaryDir "llama"
        Invoke-WebRequest -UseBasicParsing -Uri $LlamaUrl -OutFile $LlamaArchive
        Expand-Archive -Force -Path $LlamaArchive -DestinationPath $LlamaExtract
        $LlamaServer = Get-ChildItem -Path $LlamaExtract -Recurse -File -Filter "llama-server.exe" |
            Select-Object -First 1
        if (-not $LlamaServer) {
            throw "В архиве llama.cpp отсутствует llama-server.exe."
        }
        Get-ChildItem -Force $LlamaDir | Remove-Item -Recurse -Force
        Copy-Item -Recurse -Force (Join-Path $LlamaServer.Directory.FullName "*") $LlamaDir
        if (-not (Test-Path (Join-Path $LlamaDir "llama-bench.exe"))) {
            throw "В архиве llama.cpp отсутствует llama-bench.exe."
        }
        if (-not ((Test-Path (Join-Path $LlamaDir "rpc-server.exe")) -or
                  (Test-Path (Join-Path $LlamaDir "ggml-rpc-server.exe")))) {
            throw "В архиве llama.cpp отсутствует RPC server."
        }
        Set-Content -Encoding ASCII -Path (Join-Path $LlamaDir ".version") -Value $LlamaTag
    }

    $Launcher = Join-Path $BinDir "bution.cmd"
    $LauncherLines = @(
        "@echo off",
        "`"$BinDir\bution-real.exe`" --llama-bin-dir `"$LlamaDir`" %*"
    )
    Set-Content -Encoding ASCII -Path $Launcher -Value $LauncherLines

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $CurrentPaths = if ($UserPath) { $UserPath -split ";" } else { @() }
    if ($CurrentPaths -notcontains $BinDir) {
        $NewPath = if ([string]::IsNullOrWhiteSpace($UserPath)) { $BinDir } else { "$UserPath;$BinDir" }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    }
    $env:Path = "$BinDir;$env:Path"

    $FirewallReady = $false
    try {
        $TcpRule = Get-NetFirewallRule -DisplayName "BUTION TCP" -ErrorAction Stop
        $UdpRule = Get-NetFirewallRule -DisplayName "BUTION mDNS" -ErrorAction Stop
        $FirewallReady = [bool]$TcpRule -and [bool]$UdpRule
    }
    catch {
        $FirewallReady = $false
    }

    if ($FirewallReady) {
        Write-Step "правила Windows Firewall уже настроены."
    }
    else {
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
        $FirewallProcess = Start-Process powershell.exe -Verb RunAs -Wait -PassThru `
            -ArgumentList "-NoProfile", "-EncodedCommand", $Encoded
        if ($FirewallProcess.ExitCode -ne 0) {
            throw "Не удалось настроить Windows Firewall."
        }
    }

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
