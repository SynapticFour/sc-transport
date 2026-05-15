# Network Emulation (Linux + macOS)

This runbook explains how to emulate WAN quality for local `sct` transfer tests.

- Linux equivalent: `tc netem`
- macOS equivalent: `pf` + `dummynet` (`pfctl` + `dnctl`)

> Note: both Linux and macOS traffic shaping commands require root privileges.

## Single-command local transfer test

Use the `Makefile` wrapper to run receiver + sender + checksum verification in one command.

```bash
# 1GB, no shaping
make transfer-test
```

```bash
# 20GB Toronto<->Auckland profile (auto uses pf/dummynet on macOS, tc on Linux)
make transfer-test PROFILE=toronto-auckland SIZE_GB=20
```

Tunable knobs:

- `PORT` (default `7272`)
- `OUT_BASE` (default `/tmp/sct-transfer-test`)
- `RATE_MBIT`, `DELAY_MS`, `LOSS` for profile tuning
- `INTERFACE` for Linux shaping (default `lo`)

## sct-core loopback JSON bench (`bench-transfer`)

For **library-level** loopback measurements (same wiring as `sct-core` integration tests: ephemeral `HOME` for TLS material, `127.0.0.1:0`, `FileSender` + `FileReceiver`, default `SenderConfig` / `ReceiverConfig`), use the `bench-transfer` example. It prints **one JSON line** with per-iteration timings, percentiles, and **aggregate** goodput over the whole run.

**Sizing (important):** A single very large payload in this harness can stall or hit QUIC/runtime edge cases on some hosts. To keep the run stable while still moving **tens of megabytes** so handshake noise is small relative to payload, prefer **many iterations of ~1 MiB** (defaults below: 32 × 1 MiB ≈ **32 MiB** aggregate). Use **`aggregate_goodput_mbps`** as end-to-end goodput for that aggregate volume; per-iteration fields show jitter.

Optional knobs: `SC_SCT_ADAPTIVE_LOSS_HINT`, `SC_SCT_ADAPTIVE_BATCH_SIZE`, etc. **`FileSender`** (QUIC send path) always sets **`completion_first_enabled`** on **`AutopilotRuntime`**; **`SC_SCT_COMPLETION_FIRST` does not toggle that path**. The **`completion_campaign_metrics`** test uses the same default (**`completion_first_enabled: true`** only) and does not read **`SC_SCT_COMPLETION_FIRST`**. For an explicit off-vs-on KPI comparison in tests, use **`completion_first_ab`** (`crates/sct-core/tests/completion_first_ab.rs`). The **`AutopilotRuntime`** stack always runs **predictive stabilization** (forecast + damping); there is no separate env flag to disable it.

```bash
cd sc-transport
# Defaults: 32 iterations × 1 MiB ≈ 32 MiB aggregate; --release recommended
cargo run -p sct-core --example bench-transfer --release
```

Example output (representative; your numbers will vary):

```json
{"sct_core_version":"0.1.0","iterations":32,"payload_bytes":1048576,"aggregate_payload_bytes":33554432,"aggregate_wall_secs":0.276,"aggregate_goodput_mbps":973.0}
```

| Variable | Default | Meaning |
|----------|---------|---------|
| `SC_SCT_BENCH_ITERATIONS` | `32` | Number of separate transfers |
| `SC_SCT_BENCH_BYTES` | `1048576` | Application payload size per transfer |
| `SC_SCT_BENCH_TIMEOUT_SECS` | `180` | Per-iteration wall-clock limit |

For **multi-gigabyte** or WAN-shaped runs, use the `make transfer-test` / `sct-cli` flows above so the full stack and disk path match production.

## Predictive congestion stabilization (`sct-core`)

The adaptive **`AutopilotRuntime`** path (used by **`FileSender`**, **`bench-transfer`**, and the **`completion_*`** / **`prompt5_integration`** tests) layers **forecasting and control-loop damping** on top of utility-based multipath scheduling. Goal: **react less, anticipate more**—reduce oscillation and speculative overload before loss and RTT spikes dominate.

### What runs where

| Piece | Location | Role |
|-------|----------|------|
| **`PredictiveStabilizer`** | `crates/sct-core/src/adaptive/predictive.rs` | Ring buffer of **`NetworkSample`** → **`forecast_congestion`** → per-send **`PacketStabilization`**; EWMA **`ControlState`** (utility / pacing / duplication momentum); **`OscillationDetector`**; **`StabilityTelemetry`**; **`MultiTimescaleClock`** (fast ≈12 ms pacing, medium ≈120 ms speculative envelope, slow ≈1.5 s ranking bias). |
| **`PacketStabilization`** | same module | Passed into **`MultiPathScheduler::distribute_and_send`**: scales token refill, speculative duplicate caps, duplicate utility, optional **suppress speculative dup** when forecast queue growth is high. |
| **`QueueModel`** | `crates/sct-core/src/adaptive/optimization.rs` | **`queue_delay_velocity`** and **`predicted_queue_at_horizon`** so path utilities see **queue saturation ahead of a ~0.55 s horizon**, not only instantaneous delay. |
| **`HybridCongestionController`** | `adaptive/mod.rs` | **`last_rtt`** (and existing loss / gradient / variance) feed samples for forecasting. |
| **`StrategyEngine`** | `adaptive/mod.rs` | **`HysteresisThreshold`** on an aggressiveness score (enter ≈0.8, exit ≈0.5 by default) to avoid mode flapping; hard fallback to **Conservative** on very high loss or long decode delay. |

### FEC parity (encode vs wire)

**`FecEncoder::encode_block`** may append parity **`Packet`s** (`is_parity = true`) so completion-first and KPI paths stay aware of FEC. **`MultiPathScheduler::distribute_and_send`** returns immediately for **`packet.is_parity`**, so parity is **not** written to QUIC streams until a dedicated parity framing exists. The **`FileReceiver`** path decodes a **`ChunkDescriptor`** on every accepted data stream (`receiver/mod.rs`); raw XOR blobs would not parse as chunk metadata.

### Metrics after a pipeline

**`TransferMetrics`** (filled at end of **`run_pipeline`**) additionally exposes stability-oriented fields:

- `utility_oscillation_events`, `queue_overshoot_events`
- `utility_stability_ewma`, `congestion_forecast_confidence`, `congestion_recovery_ticks`

Use them when comparing **`completion_first_ab`** runs, **`completion_campaign_metrics`** snapshots, or custom benches—not only peak goodput.

### Tuning

Timescales, forecast blend (≈200–1000 ms effective horizon), hysteresis defaults, and suppression thresholds are **constants in Rust** (`predictive.rs`, `MultiPathScheduler`). There are **no extra `SC_SCT_*` env vars** for the stabilizer today; the same completion-first / loss-hint / batch knobs as in the table above still apply.

### Related commands

Same as elsewhere in this doc: **`bench-transfer`**, **`completion_campaign`**, and **`bash scripts/test_sct_core_integration.sh`** all exercise **`AutopilotRuntime`** with stabilization enabled.

## Linux quick start (`tc netem`)

Use the built-in matrix command:

```bash
cargo run -p sct-bench -- netem-matrix \
  --interface lo \
  --profile all \
  --sizes-mib 1,16,256,1024 \
  --output-json docs/RESULTS/netem-matrix.json
```

Copy/paste example for a single large transfer on Linux:

```bash
sudo make transfer-test PROFILE=toronto-auckland SIZE_GB=20 INTERFACE=lo
```

## macOS equivalent (`pf` + `dnctl`)

macOS does not provide `tc netem`. The closest native alternative is:

- `dnctl pipe ... config ...` for delay/loss/rate
- `pfctl` rules to attach UDP traffic to that pipe

### 1) Create a temporary anchor

```bash
cat > /tmp/sct-netem-anchor.conf <<'EOF'
dummynet out quick proto udp from any to any port 7272 pipe 1
dummynet in  quick proto udp from any to any port 7272 pipe 1
EOF
```

### 2) Enable pipe profile + rules

Toronto <-> Auckland is usually long-haul intercontinental:
- RTT: often around 180-260 ms
- jitter: variable
- packet loss: low but non-zero in real paths

The profile below is a practical approximation:

```bash
# Pipe 1: bandwidth + one-way delay + packet loss ratio (plr)
sudo dnctl pipe 1 config bw 80Mbit/s delay 110ms plr 0.008

# Load anchor rules and enable PF
sudo pfctl -a com.synapticfour.sct -f /tmp/sct-netem-anchor.conf
sudo pfctl -e
```

This results in approximately:
- ~220 ms RTT (110 ms each direction)
- 0.8% loss
- 80 Mbit/s bottleneck

### 3) Run a 20 GB transfer test

Terminal A (receiver):

```bash
cargo run -p sct-cli -- recv --port 7272 --output-dir /tmp/sct-recv --once
```

Terminal B (sender):

```bash
mkdir -p /tmp/sct-send /tmp/sct-recv
mkfile -n 20g /tmp/sct-send/payload-20g.bin

time cargo run -p sct-cli -- send /tmp/sct-send/payload-20g.bin sct://127.0.0.1:7272 --compression none --quiet
```

Or as one copy/paste command:

```bash
make transfer-test PROFILE=toronto-auckland SIZE_GB=20 PORT=7272 OUT_BASE=/tmp/sct-transfer-test
```

Optional integrity check:

```bash
shasum -a 256 /tmp/sct-send/payload-20g.bin /tmp/sct-recv/payload-20g.bin
```

### 4) Cleanup (important)

```bash
sudo pfctl -a com.synapticfour.sct -F rules
sudo dnctl -q flush
sudo pfctl -d
rm -f /tmp/sct-netem-anchor.conf
```

## Additional informative profile examples

Replace only the `dnctl pipe` command, keeping the same anchor and `sct` commands.

### Good metro / regional

```bash
sudo dnctl pipe 1 config bw 900Mbit/s delay 6ms plr 0.0001
```

- Approx: 12 ms RTT, near-zero loss, high capacity
- Single command:

```bash
make transfer-test PROFILE=toronto-auckland SIZE_GB=5 RATE_MBIT=900 DELAY_MS=6 LOSS=0.0001
```

### Busy continental

```bash
sudo dnctl pipe 1 config bw 200Mbit/s delay 35ms plr 0.002
```

- Approx: 70 ms RTT, 0.2% loss
- Single command:

```bash
make transfer-test PROFILE=toronto-auckland SIZE_GB=10 RATE_MBIT=200 DELAY_MS=35 LOSS=0.002
```

### Very degraded long-haul

```bash
sudo dnctl pipe 1 config bw 20Mbit/s delay 150ms plr 0.03
```

- Approx: 300 ms RTT, 3% loss, constrained throughput
- Single command:

```bash
make transfer-test PROFILE=toronto-auckland SIZE_GB=2 RATE_MBIT=20 DELAY_MS=150 LOSS=0.03
```

## CI suggestion

For CI on Linux runners, prefer `sct-bench netem-matrix`.

For CI on macOS runners:
- either run unshaped synthetic benchmarks only, or
- use privileged runners and execute the `pfctl`/`dnctl` setup before transfer tests.

## Streaming-only matrix (no large files)

Outputs default to **`results/`** at the repo root (gitignored; see `results/README.md`).
Use **`docs/RESULTS/`** for committed measurement write-ups.

For low-disk environments, run the streaming matrix integration test:

```bash
SC_TRANSPORT_ARTIFACT_DIR=results \
SC_STREAM_MATRIX_REPEATS=2 \
cargo test --test streaming_matrix --features "transport-quic transport-datagrams"
```

Focused smoke (**excellent** quality, **tiny** and **small** payloads only; good for checking localhost stability without running the full cross-product):

```bash
SC_STREAM_MATRIX_QUALITIES=excellent \
SC_STREAM_MATRIX_PAYLOADS=tiny,small \
SC_STREAM_MATRIX_REPEATS=3 \
cargo test --test streaming_matrix --features "transport-quic transport-datagrams"
```

Optional filters (comma-separated, names must match the built-in profiles):

- **`SC_STREAM_MATRIX_QUALITIES`**: `excellent`, `good`, `poor`, `very-poor` (omit for all).
- **`SC_STREAM_MATRIX_PAYLOADS`**: `tiny`, `small`, `medium`, `large` (omit for all).

If every selected quality has **0%** simulated loss, the integration test **does not** require a `quic-datagram` fallback sample (that assertion only runs when at least one selected profile can inject loss).

Closed-loop A/B (same repeats for fair comparison):

```bash
# A: closed-loop off
SC_TRANSPORT_ARTIFACT_DIR=results/A \
SC_STREAM_MATRIX_REPEATS=5 \
SC_STREAM_MATRIX_CLOSED_LOOP=0 \
cargo test --test streaming_matrix --features "transport-quic transport-datagrams"

# B: closed-loop on
SC_TRANSPORT_ARTIFACT_DIR=results/B \
SC_STREAM_MATRIX_REPEATS=5 \
SC_STREAM_MATRIX_CLOSED_LOOP=1 \
cargo test --test streaming_matrix --features "transport-quic transport-datagrams"
```

The summary now also includes:

- `mode-switch-count total`
- `effective-parity-rate p50`
- `feedback-lag-ms p95`
- `duplication-rate p50`
- `deadline-miss-rate p95`
- `tail-ratio p99/p50`

Produced artifacts:

- `results/streaming-matrix.json` (raw per-run cases)
- `results/streaming-matrix-summary.json` (p50/p95 aggregated summary)

Before/after comparison:

```bash
python3 scripts/compare_streaming_matrix.py \
  --before results/streaming-matrix-summary-before.json \
  --after results/streaming-matrix-summary.json \
  --out-md results/streaming-matrix-compare.md \
  --out-json results/streaming-matrix-compare.json
```

Tip: keep the same `SC_STREAM_MATRIX_REPEATS` value for before and after runs to reduce comparison noise.

## Competitive baseline matrix (free alternatives)

Generate a fair side-by-side matrix against free baseline transports:

- `baseline-tcp` (raw TCP streaming socket)
- `baseline-udp` (raw UDP datagram socket)
- `sse`, `quic-stream`, `quic-datagram` (from `sc-transport`)

Run:

```bash
python3 scripts/competitive_baseline_matrix.py \
  --sc-summary results/streaming-matrix-summary-B.json \
  --repeats 5 \
  --out-json results/competitive-baseline-matrix.json \
  --out-md results/competitive-baseline-matrix.md
```

Notes:

- This matrix is streaming-only and disk-light.
- For meaningful optimization decisions, focus primarily on `good`, `poor`, and `very-poor` profiles.
- `excellent` on localhost can be noisy due to scheduler/runtime burst effects.

## Completion-first campaign (sct-core)

**`completion_campaign_metrics`** writes **`completion-campaign-summary.json`** when **`SC_TRANSPORT_ARTIFACT_DIR`** is set. The summary always has **`completion_first: true`** (aligned with **`FileSender`**). Relative paths under **`SC_TRANSPORT_ARTIFACT_DIR`** are resolved from the **workspace root** (integration tests’ cwd is **`crates/sct-core`**).

Cf-check snapshot (overwrites **`results/cf-check/completion-campaign-summary.json`**):

```bash
cd sc-transport
SC_TRANSPORT_ARTIFACT_DIR=results/cf-check \
  cargo test -p sct-core --test completion_campaign completion_campaign_metrics -- --exact
```

**A/B off vs on** for completion-first behavior is exercised by **`completion_first_ab`** (single test: baseline **`false`** vs completion **`true`**), not by toggling env for **`completion_campaign`**.

To compare two saved JSON summaries (e.g. archived **`results/cf-A`** / **`results/cf-B`** from older runs where **`completion_first`** differed, or any two files you maintain):

```bash
python3 scripts/compare_completion_campaign.py \
  --before results/cf-A/completion-campaign-summary.json \
  --after results/cf-B/completion-campaign-summary.json \
  --out-json results/completion-campaign-compare.json \
  --out-md results/completion-campaign-compare.md
```

## sct-core: Integrationstests zuverlässig ausführen

Mehrere QUIC-Loopback-Tests unter `crates/sct-core/tests/` starten als **eigene Test-Binaries**. Läuft `cargo test -p sct-core` ohne Einschränkung, können sie **parallel** laufen und sich gegenseitig stören (lange Laufzeit oder „Hängen“).

Empfehlungen:

1. **Sequentiell (robust):**  
   `bash scripts/test_sct_core_integration.sh`  
   (setzt `RUST_TEST_THREADS=1` und führt nacheinander `cargo test -p sct-core --test <name>` aus.)

   Alternativ: `cargo test-sct-core` (Alias in `.cargo/config.toml` — serialisiert vor allem den Harness **innerhalb** eines Test-Binaries; für harte Isolation das Shell-Script bevorzugen.)

2. **Timeouts:** Die Tests `prompt5_integration`, `transfer_smoke`, `completion_*` nutzen `tests/common.rs` → `with_timeout`: bei echtem Deadlock schlägt der Test nach spätestens 60–300 s fehl statt endlos zu laufen.

3. **Nur Bibliothek (schnell):**  
   `cargo test -p sct-core --lib`

4. **Sehr langer Workspace-Lauf:** `cargo test --workspace` baut u.a. **`sc-transport-datagrams`** mit; der Test **`transparency_final_state_matches_across_transports`** kann unter Parallelität **viele Minuten** dauern (nicht „hängend“, nur langsam). Nur diesen Test isoliert:  
   `cargo test -p sc-transport-datagrams transparency_final_state_matches_across_transports`
