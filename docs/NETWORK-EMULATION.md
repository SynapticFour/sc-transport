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

For low-disk environments, run the streaming matrix integration test:

```bash
SC_TRANSPORT_ARTIFACT_DIR=results \
SC_STREAM_MATRIX_REPEATS=2 \
cargo test --test streaming_matrix --features "transport-quic transport-datagrams"
```

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

Run A/B for block completion KPIs:

```bash
# A: completion-first off
SC_TRANSPORT_ARTIFACT_DIR=results/cf-A \
SC_SCT_COMPLETION_FIRST=0 \
cargo test -p sct-core --test completion_campaign completion_campaign_metrics

# B: completion-first on
SC_TRANSPORT_ARTIFACT_DIR=results/cf-B \
SC_SCT_COMPLETION_FIRST=1 \
cargo test -p sct-core --test completion_campaign completion_campaign_metrics
```

Compare:

```bash
python3 scripts/compare_completion_campaign.py \
  --before results/cf-A/completion-campaign-summary.json \
  --after results/cf-B/completion-campaign-summary.json \
  --out-json results/completion-campaign-compare.json \
  --out-md results/completion-campaign-compare.md
```
