# BUTION architecture

## Data flow

```text
mDNS discovery
      │
      ▼
Noise XX handshake ── pairing approval ── persistent UUID/key trust
      │
      ▼
encrypted control channel
      ├── hardware and available-memory query
      ├── temporary latency/bandwidth server
      └── start/stop RPC worker on selected LAN address
      │
      ▼
Main: GGUF fit → route score → tensor split → llama-server --rpc
      │
      ▼
127.0.0.1 OpenAI-compatible SSE → terminal chat + telemetry
```

## Modules

| Module | Responsibility |
| --- | --- |
| `cluster` | Serializable roles, node summaries, and control messages |
| `discovery` | `_bution._tcp.local.` publication and mDNS resolution |
| `security` | Ed25519 identity utilities, Noise static keys, encrypted framing |
| `control` | Pairing decisions and authenticated worker/benchmark commands |
| `hardware` | Cross-platform CPU, RAM, architecture, and backend inventory |
| `hub` | Hugging Face search, GGUF filtering/ranking, resumable streaming downloads |
| `network` | Interface filtering, reachability, measurement, and scoring |
| `models` | GGUF header validation, RAM estimation, local/cluster fit |
| `optimizer` | Capped weighted allocation and measured split search/cache |
| `llama` | Binary discovery and shell-free argument construction |
| `processes` | Child lifecycle, output capture, crash state, and cleanup |
| `runtime` | Async orchestration of discovery through model startup |
| `chat` | Incremental SSE decoder and local llama-server client |
| `telemetry` | Per-second RAM, CPU, network, and token-speed sampling |
| `tui` | Ratatui rendering, navigation, pairing dialog, and chat editor |

Hub HTTP never runs in the rendering/input path. Search, repository inspection,
download, cancellation, validation, and deletion communicate with the TUI through
bounded command/event channels. Downloads are streamed to `.gguf.part`, checked
against free disk space, resumed with HTTP Range when supported, and renamed only
after their size and existing `models` GGUF validation pass.

## Distribution policy

The scheduler first proves that summed safe memory can hold the estimated model
footprint. Each node receives a priority derived from usable memory, relative
compute performance, and network score. A capped weighted allocation repeatedly
redistributes excess from a node that would exceed its memory capacity. The
result is passed to llama.cpp in device order as `--tensor-split`.

For a two-node optimization run, memory-invalid candidates are removed before
testing 80/20, 70/30, 60/40, and 50/50. `llama-bench -o json` provides prompt
processing and generation throughput. Highest generation tok/s wins, with TTFT
as a tie-breaker. Cache keys include model, hardware, and network fingerprints.

## Process ownership

BUTION launches executables directly with argument arrays, never through a
shell. Standard output and error are captured into structured log events. Each
managed child has kill-on-drop enabled. Normal shutdown asks remote workers to
stop and then reaps all local children; abnormal parent drop still kills local
children.

## MVP boundaries

The runtime path currently selects one worker, while protocol and allocation
types support multiple nodes. GPU utilization telemetry is backend-specific and
is shown as unavailable when no portable source exists. The inference engine,
tensor transfer, and computation remain entirely inside upstream llama.cpp RPC.
