# BUTION

```text
██████╗ ██╗   ██╗████████╗██╗ ██████╗ ███╗   ██╗
██╔══██╗██║   ██║╚══██╔══╝██║██╔═══██╗████╗  ██║
██████╔╝██║   ██║   ██║   ██║██║   ██║██╔██╗ ██║
██╔══██╗██║   ██║   ██║   ██║██║   ██║██║╚██╗██║
██████╔╝╚██████╔╝   ██║   ██║╚██████╔╝██║ ╚████║
╚═════╝  ╚═════╝    ╚═╝   ╚═╝ ╚═════╝ ╚═╝  ╚═══╝
```

**Distributed Local AI Cluster**

BUTION joins trusted computers on a local network so a GGUF model can run with
the `llama.cpp` RPC backend when it does not fit in one machine's memory. It is a
keyboard-driven terminal application written in Rust. It does **not** emulate
shared operating-system RAM.

The MVP targets two computers (for example, a 16 GiB Apple Silicon Mac and a
16 GiB Windows laptop), with an extensible protocol and optimizer for more nodes.

## What works

- mDNS/Bonjour discovery with a permanent node UUID;
- first-use pairing and a persistent UUID + public-key trust store;
- encrypted Noise XX control channel (X25519, ChaChaPoly, BLAKE2s);
- OS, CPU, core count, memory, Metal/CUDA/Vulkan capability detection;
- conservative AI memory budget: current availability capped by
  `total RAM - max(2 GiB, 15%)`;
- enumeration of Wi-Fi, Ethernet, USB/Thunderbolt and private LAN addresses;
- automatic VPN exclusion, TCP latency/jitter/bandwidth benchmark and route score;
- managed `rpc-server`/`ggml-rpc-server`, `llama-server`, and `llama-bench`
  processes with captured logs and cleanup;
- GGUF validation, memory fit recommendation, resource-aware tensor split and
  optional 80/20–50/50 optimization API;
- OpenAI-compatible streaming chat through the local `llama-server`;
- live RAM, CPU, network, and generation-speed telemetry;
- ratatui dashboard with Cluster, Nodes, Models, Benchmark, Chat, and Settings.

## Requirements

- Rust 1.85 or newer;
- Git and CMake 3.20 or newer for building `llama.cpp`;
- the same recent `llama.cpp` RPC-enabled build on both computers;
- both computers on the same trusted LAN. A phone hotspot plus a direct Ethernet
  cable is supported; BUTION benchmarks direct routes and prefers the best one.

Do not expose RPC ports to the internet. Do not disable the operating-system
firewall. Allow only the private-network rules described below.

## Build BUTION

```bash
cargo build --release
cargo test
```

The binary is `target/release/bution` on macOS/Linux and
`target\release\bution.exe` on Windows.

## Build llama.cpp on macOS

Install Xcode Command Line Tools and CMake, then run:

```bash
./scripts/build-llama-macos.sh
```

This enables Metal and `GGML_RPC`. The script prints the resulting binary
directory, normally `.bution/llama.cpp/build/bin`.

## Build llama.cpp on Windows

Install Visual Studio 2022 Build Tools with C++, Git, and CMake. In PowerShell:

```powershell
.\scripts\build-llama-windows.ps1 -Backend CPU
```

Use `-Backend Vulkan` for a supported Vulkan GPU or `-Backend CUDA` when the
CUDA toolkit is installed. The normal output directory is
`.bution\llama.cpp\build\bin\Release`.

The current upstream executable is named `rpc-server`; BUTION also recognizes
the older `ggml-rpc-server` name.

## Run on both computers

Start the worker-capable machine first:

```powershell
.\target\release\bution.exe `
  --llama-bin-dir .bution\llama.cpp\build\bin\Release
```

Start the computer holding the GGUF model:

```bash
./target/release/bution \
  --llama-bin-dir .bution/llama.cpp/build/bin \
  --model /absolute/path/to/model.gguf
```

`Automatic` is the default role. The computer where Enter is pressed on the
Cluster screen becomes Main; a paired peer supplies the RPC device when the
model does not fit locally.

On first contact, the receiving computer shows a six-digit pairing code. Verify
the expected machine name and LAN address, select **Accept**, and press Enter.
The static Noise public key is pinned to the permanent UUID for later sessions.

On the Cluster screen, press Enter to start the selected model. BUTION will:

1. inspect the GGUF and calculate required memory;
2. query the paired worker's resources;
3. benchmark every directly reachable non-VPN LAN path;
4. ask the worker to bind RPC only to the selected LAN address;
5. calculate a memory-safe tensor split;
6. launch local `llama-server` on `127.0.0.1:8080`;
7. stream chat responses in the Chat screen.

Press Enter on Cluster again to stop the model. Pressing Q anywhere outside the
chat editor exits and cleans up managed processes on both nodes.

## Keyboard controls

| Key | Action |
| --- | --- |
| `↑` / `↓` | move through sections |
| `←` / `→` | switch sections or pairing choice |
| `Enter` | start/stop model, send chat, confirm pairing |
| `Esc` | back; reject an open pairing prompt |
| `Space` | toggle setting or pairing choice |
| `Q` | exit outside chat |
| `Ctrl+N` | new chat |
| `Ctrl+L` | clear chat |

## Firewall ports

Restrict these rules to the trusted private network profile:

| Port | Use |
| --- | --- |
| UDP 5353 | mDNS/Bonjour discovery |
| TCP 31750 | encrypted BUTION control channel |
| TCP 31751 | temporary network benchmark (opened after pairing) |
| TCP 50052 | llama.cpp RPC worker (opened after pairing) |

The model HTTP API listens only on `127.0.0.1:8080` on Main. BUTION never
modifies firewall configuration automatically.

## Troubleshooting

- **No nodes appear:** confirm both devices are on the same private LAN, mDNS is
  allowed, and client isolation is disabled on the hotspot/router.
- **llama.cpp binaries not found:** pass the directory containing all three
  executables with `--llama-bin-dir` on both computers.
- **No direct LAN route:** verify that Ethernet/Thunderbolt addresses are in the
  same subnet. VPN interfaces are deliberately ignored.
- **Model cannot start:** inspect the event log. The GGUF estimate includes 13%
  runtime overhead plus 1 GiB; cluster available memory must exceed it.
- **Pairing key changed:** remove the stale trusted peer from the local settings
  only after verifying that the remote installation was intentionally reset.

Settings, identities, trust records, and optimization cache use the operating
system's per-user application-data directories. Private identity files are mode
`0600` on Unix.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

See [Architecture](docs/ARCHITECTURE.md) and [Security](docs/SECURITY.md) for
protocol and trust-boundary details.

The repository intentionally has no configured remote. Add your own `origin`
when ready.
