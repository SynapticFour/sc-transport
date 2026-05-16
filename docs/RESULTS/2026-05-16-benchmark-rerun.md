# Benchmark rerun — 2026-05-16

Full matrix re-run on loopback after commit `e262c77` (post–CI-fix).

## Setup

- Commit: `e262c778f6b0691be089e8107851711f1c94f5b6`
- Host: macOS, release build, `RUST_TEST_THREADS=1`, `repeats=5`
- Command: `bash scripts/run-full-benchmark-rerun.sh` (`RUN_ID=rerun-2026-05-16`)
- Streaming matrix: 4 qualities × 4 payloads × 3 transports (48 summary cells)
- Netem matrix: skipped (requires Linux + root)

## Highlights (excellent, loopback)

| Szenario | I-final | rerun-2026-05-16 | Δ |
|---|---|---|---|
| quic-stream/excellent/large | 17 237 eps | 12 940 eps | −24.9% |
| quic-stream/excellent/small | 25 113 eps | 29 345 eps | +16.9% |
| sse/excellent/large | 72 639 eps | 52 597 eps | −27.6% |
| sse/excellent/small | 265 986 eps | 274 066 eps | +3.0% |
| quic-datagram/excellent/large (fallback) | 200/200 | 200/200 | — |
| quic-datagram/excellent/small (fallback) | 200/200 | 200/200 | — |
| cf-check p99 | 0.015 ms | 0.011 ms | −26% |

WAN-shaped profiles (`good` / `poor` / `very-poor`) match I-final within ~1–2% (≈350 / ≈120 / ≈67 eps).

## Other runs in same batch

| Step | Result |
|------|--------|
| Competitive baseline | Generated from rerun summary |
| Compare vs G | `results/streaming-matrix-compare-G-vs-rerun-2026-05-16.md` (local) |
| Compare vs I-final | `results/streaming-matrix-compare-I-final-vs-rerun-2026-05-16.md` (local) |
| sct-bench synthetic | ~16 949 Gbps loopback (64 MiB × 5 samples) |

## Raw artifacts (gitignored)

Local tree: `results/rerun-2026-05-16/`

- `streaming-matrix-summary.json`
- `competitive-baseline-matrix.md`
- `cf-check/completion-campaign-summary.json`

## Interpretation

Excellent-profile eps on localhost vary ±25% run-to-run; **poor** and **very-poor** are the stable regression anchors. No change to datagram fallback policy (large/small payloads still route to SSE by design).

Re-run anytime:

```bash
bash scripts/run-full-benchmark-rerun.sh
```
