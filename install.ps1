$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
try {
    [Console]::OutputEncoding = [System.Text.Encoding]::UTF8
}
catch {
    # ignore if console encoding cannot be changed
}

function Show-Banner {
    Write-Host ""
    Write-Host "  ____  _   _ _____ ___ ___  _   _ " -ForegroundColor Cyan
    Write-Host " | __ )| | | |_   _|_ _/ _ \| \ | |" -ForegroundColor Cyan
    Write-Host " |  _ \| | | | | |  | | | | |  \| |" -ForegroundColor Cyan
    Write-Host " | |_) | |_| | | |  | | |_| | |\  |" -ForegroundColor Cyan
    Write-Host " |____/ \___/  |_| |___\___/|_| \_|" -ForegroundColor Cyan
    Write-Host ""
    Write-Host " ⚡ BUTION" -ForegroundColor Yellow -NoNewline
    Write-Host " — распределённый запуск LLM в локальной сети" -ForegroundColor White
    Write-Host "─────────────────────────────────────────────────────────────" -ForegroundColor DarkGray
    Write-Host ""
}

function Write-StepHeader([string]$Num, [string]$Title) {
    Write-Host "[$Num] " -ForegroundColor Blue -NoNewline
    Write-Host $Title -ForegroundColor White
}

function Write-Success([string]$Message) {
    Write-Host "       ✔ " -ForegroundColor Green -NoNewline
    Write-Host $Message -ForegroundColor Gray
}

function Write-Info([string]$Message) {
    Write-Host "       ℹ " -ForegroundColor DarkCyan -NoNewline
    Write-Host $Message -ForegroundColor DarkGray
}

function Write-Fail([string]$Message) {
    Write-Host "`n✖ Ошибка: " -ForegroundColor Red -NoNewline
    Write-Host $Message -ForegroundColor White
    throw $Message
}

function Download-FileWithProgress([string]$Url, [string]$Destination) {
    $request = [System.Net.HttpWebRequest]::Create($Url)
    $request.UserAgent = "BUTION-Installer"
    $request.Timeout = 60000
    $request.AllowAutoRedirect = $true

    $response = $request.GetResponse()
    $totalBytes = $response.ContentLength
    $responseStream = $response.GetResponseStream()
    $fileStream = [System.IO.File]::Create($Destination)

    $buffer = New-Object byte[] 65536
    $downloadedBytes = 0
    $lastUpdate = [System.Diagnostics.Stopwatch]::StartNew()
    $barWidth = 26
    $isInteractive = [Environment]::UserInteractive -and (-not [Console]::IsOutputRedirected)

    try {
        while (($bytesRead = $responseStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
            $fileStream.Write($buffer, 0, $bytesRead)
            $downloadedBytes += $bytesRead

            if ($isInteractive -and ($lastUpdate.ElapsedMilliseconds -gt 70 -or $downloadedBytes -ge $totalBytes)) {
                $lastUpdate.Restart()
                if ($totalBytes -gt 0) {
                    $percent = [Math]::Min(100, [int](($downloadedBytes / $totalBytes) * 100))
                    $filled = [int](($percent / 100) * $barWidth)
                    $empty = $barWidth - $filled
                    $bar = ("█" * $filled) + ("░" * $empty)
                    $currMb = ($downloadedBytes / 1MB).ToString("0.0")
                    $totalMb = ($totalBytes / 1MB).ToString("0.0")
                    $line = "`r       [$bar] $percent% ($currMb / $totalMb MB)  "
                    Write-Host -NoNewline $line -ForegroundColor DarkCyan
                }
                else {
                    $currMb = ($downloadedBytes / 1MB).ToString("0.0")
                    $line = "`r       Загружено: $currMb MB  "
                    Write-Host -NoNewline $line -ForegroundColor DarkCyan
                }
            }
        }
    }
    finally {
        $fileStream.Close()
        $responseStream.Close()
        $response.Close()
    }

    if ($isInteractive) {
        Write-Host "`r                                                                      `r" -NoNewline
    }
}

function Get-Utf8WebString([string]$Url) {
    $request = [System.Net.HttpWebRequest]::Create($Url)
    $request.UserAgent = "BUTION-Installer"
    $request.Timeout = 15000
    $request.AllowAutoRedirect = $true

    $response = $request.GetResponse()
    $stream = $response.GetResponseStream()
    $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
    $text = $reader.ReadToEnd()
    $reader.Close()
    $stream.Close()
    $response.Close()
    return $text.TrimStart([char]0xFEFF).Trim()
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

Show-Banner

$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -notin @("AMD64", "x86_64")) {
    Write-Fail "Этот установщик пока поддерживает Windows x64."
}

$Repo = "danytebyya/bution"
$InstallDir = Join-Path $env:LOCALAPPDATA "BUTION"
$BinDir = Join-Path $InstallDir "bin"
$LlamaDir = Join-Path $InstallDir "llama"
$ForceUpdate = $env:BUTION_FORCE_UPDATE -eq "1"
$TemporaryDir = Join-Path ([IO.Path]::GetTempPath()) ("bution-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $BinDir, $LlamaDir, $TemporaryDir | Out-Null

try {
    # 1/4: BUTION
    Write-StepHeader "1/4" "Загрузка BUTION…"
    $ButionBinary = Join-Path $BinDir "bution-real.exe"
    $ButionReady = (Test-Path $ButionBinary) -and ((Get-Item $ButionBinary).Length -gt 0)
    if ((-not $ForceUpdate) -and $ButionReady) {
        Write-Success "BUTION уже установлен ($ButionBinary)"
    }
    else {
        $ButionArchive = Join-Path $TemporaryDir "bution.zip"
        $ButionExtract = Join-Path $TemporaryDir "bution-extract"
        $ButionUrl = "https://github.com/$Repo/releases/latest/download/bution-windows-x64.zip"
        $ButionReady = $false
        try {
            Download-FileWithProgress $ButionUrl $ButionArchive
            Expand-Archive -Force -Path $ButionArchive -DestinationPath $ButionExtract
            $ButionExe = Get-ChildItem -Path $ButionExtract -Recurse -File -Filter "bution.exe" |
                Select-Object -First 1
            if ($ButionExe) {
                Copy-Item -Force $ButionExe.FullName $ButionBinary
                $ButionReady = $true
                Write-Success "BUTION успешно загружен и установлен"
            }
        }
        catch {
            Write-Info "Готовый релиз недоступен — выполняю автоматическую сборку…"
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
                Write-Success "Microsoft C++ Build Tools уже установлены"
            }
            else {
                if (-not (Get-Command winget.exe -ErrorAction SilentlyContinue)) {
                    Write-Fail "Для автоматической сборки нужен winget (Microsoft App Installer)."
                }
                Write-Info "Устанавливаю Microsoft C++ Build Tools (может занять 10–30 минут)…"
                & winget.exe install --id Microsoft.VisualStudio.2022.BuildTools --exact --silent `
                    --accept-source-agreements --accept-package-agreements --disable-interactivity `
                    --override "--wait --quiet --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
                if ($LASTEXITCODE -ne 0) {
                    Write-Fail "Не удалось автоматически установить Microsoft C++ Build Tools."
                }
                Write-Success "Microsoft C++ Build Tools успешно установлены"
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
                Write-Success "Rust уже установлен"
            }
            else {
                Write-Info "Устанавливаю Rust…"
                $Rustup = Join-Path $TemporaryDir "rustup-init.exe"
                Download-FileWithProgress "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe" $Rustup
                & $Rustup -y --profile minimal --default-toolchain stable
                if ($LASTEXITCODE -ne 0) {
                    Write-Fail "Не удалось автоматически установить Rust."
                }
                Write-Success "Rust успешно установлен"
            }
            $CargoDir = Split-Path $Cargo
            if ($CargoDir) {
                $env:Path = "$CargoDir;$env:Path"
            }

            Write-Info "Скачиваю исходники BUTION…"
            $SourceArchive = Join-Path $TemporaryDir "source.zip"
            $SourceExtract = Join-Path $TemporaryDir "source"
            Download-FileWithProgress "https://github.com/$Repo/archive/refs/heads/main.zip" $SourceArchive
            Expand-Archive -Force -Path $SourceArchive -DestinationPath $SourceExtract
            $Manifest = Get-ChildItem -Path $SourceExtract -Recurse -File -Filter "Cargo.toml" |
                Sort-Object { $_.FullName.Length } |
                Select-Object -First 1
            if (-not $Manifest) {
                Write-Fail "В архиве BUTION отсутствует Cargo.toml."
            }
            Write-Info "Компиляция BUTION (release)…"
            & $Cargo build --release --locked --manifest-path $Manifest.FullName
            if ($LASTEXITCODE -ne 0) {
                Write-Fail "Автоматическая сборка BUTION завершилась ошибкой."
            }
            $BuiltBinary = Join-Path $Manifest.Directory.FullName "target\release\bution.exe"
            Copy-Item -Force $BuiltBinary $ButionBinary
            Write-Success "BUTION успешно собран из исходников"
        }
    }

    Write-Host ""

    # 2/4: llama.cpp
    Write-StepHeader "2/4" "Загрузка llama.cpp с RPC…"
    $LlamaReady = (Test-Path (Join-Path $LlamaDir "llama-server.exe")) -and
        (Test-Path (Join-Path $LlamaDir "llama-bench.exe")) -and
        ((Test-Path (Join-Path $LlamaDir "rpc-server.exe")) -or
         (Test-Path (Join-Path $LlamaDir "ggml-rpc-server.exe")))
    if ((-not $ForceUpdate) -and $LlamaReady) {
        $VersionFile = Join-Path $LlamaDir ".version"
        $InstalledVer = if (Test-Path $VersionFile) { "сборка " + (Get-Content $VersionFile).Trim() } else { "актуальная версия" }
        Write-Success "llama.cpp с RPC уже установлен ($InstalledVer)"
    }
    else {
        $LlamaTagUrl = "https://github.com/ggml-org/llama.cpp/releases/download/v0.3.0/nightly-tag.txt"
        $LlamaTag = try {
            Get-Utf8WebString $LlamaTagUrl
        }
        catch {
            $LlamaTagResp = Invoke-WebRequest -UseBasicParsing -Uri $LlamaTagUrl
            ConvertTo-Utf8Text $LlamaTagResp.Content
        }
        if ($LlamaTag -notmatch "^b[0-9]+$") {
            Write-Fail "Не удалось определить актуальную сборку llama.cpp: '$LlamaTag'."
        }
        Write-Info "Актуальная сборка llama.cpp: $LlamaTag"
        $LlamaAsset = "llama-$LlamaTag-bin-win-cpu-x64.zip"
        $LlamaUrl = "https://github.com/ggml-org/llama.cpp/releases/download/$LlamaTag/$LlamaAsset"
        $LlamaArchive = Join-Path $TemporaryDir "llama.zip"
        $LlamaExtract = Join-Path $TemporaryDir "llama"
        Download-FileWithProgress $LlamaUrl $LlamaArchive
        Expand-Archive -Force -Path $LlamaArchive -DestinationPath $LlamaExtract
        $LlamaServer = Get-ChildItem -Path $LlamaExtract -Recurse -File -Filter "llama-server.exe" |
            Select-Object -First 1
        if (-not $LlamaServer) {
            Write-Fail "В архиве llama.cpp отсутствует llama-server.exe."
        }
        Get-ChildItem -Force $LlamaDir | Remove-Item -Recurse -Force
        Copy-Item -Recurse -Force (Join-Path $LlamaServer.Directory.FullName "*") $LlamaDir
        if (-not (Test-Path (Join-Path $LlamaDir "llama-bench.exe"))) {
            Write-Fail "В архиве llama.cpp отсутствует llama-bench.exe."
        }
        if (-not ((Test-Path (Join-Path $LlamaDir "rpc-server.exe")) -or
                  (Test-Path (Join-Path $LlamaDir "ggml-rpc-server.exe")))) {
            Write-Fail "В архиве llama.cpp отсутствует RPC server."
        }
        Set-Content -Encoding ASCII -Path (Join-Path $LlamaDir ".version") -Value $LlamaTag
        Write-Success "llama.cpp с RPC успешно установлен (сборка $LlamaTag)"
    }

    Write-Host ""

    # 3/4: Launcher & PATH
    Write-StepHeader "3/4" "Настройка лаунчера и переменной окружения PATH…"
    $Launcher = Join-Path $BinDir "bution.cmd"
    $LauncherLines = @(
        "@echo off",
        "`"$BinDir\bution-real.exe`" --llama-bin-dir `"$LlamaDir`" %*"
    )
    Set-Content -Encoding ASCII -Path $Launcher -Value $LauncherLines
    Write-Success "Лаунчер bution.cmd настроен"

    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $CurrentPaths = if ($UserPath) { $UserPath -split ";" } else { @() }
    if ($CurrentPaths -notcontains $BinDir) {
        $NewPath = if ([string]::IsNullOrWhiteSpace($UserPath)) { $BinDir } else { "$UserPath;$BinDir" }
        [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    }
    $env:Path = "$BinDir;$env:Path"
    Write-Success "Команда bution добавлена в PATH"

    Write-Host ""

    # 4/4: Windows Firewall
    Write-StepHeader "4/4" "Настройка Windows Firewall для частной сети…"
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
        Write-Success "Правила Windows Firewall уже настроены"
    }
    else {
        Write-Info "Запрос прав администратора для настройки Firewall…"
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
            Write-Fail "Не удалось настроить Windows Firewall."
        }
        Write-Success "Правила Windows Firewall для TCP (31750, 31751, 50052) и UDP (5353) созданы"
    }

    Write-Host ""
    Write-Host "─────────────────────────────────────────────────────────────" -ForegroundColor DarkGray
    Write-Host " ✨ Установка успешно завершена!" -ForegroundColor Green
    Write-Host ""
    Write-Host " Запустить прямо сейчас:" -ForegroundColor White
    Write-Host "   $Launcher" -ForegroundColor Cyan
    Write-Host ""
    Write-Host " В новом окне PowerShell можно использовать просто:" -ForegroundColor White
    Write-Host "   bution" -ForegroundColor Yellow
    Write-Host ""
}
finally {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TemporaryDir
}
