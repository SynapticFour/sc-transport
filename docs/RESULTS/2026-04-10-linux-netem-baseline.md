# Linux Netem Baseline Report

Date: 2026-04-10  
Scope: `sc-transport` prompt 1-5 production-hard closeout  
Host profile: Linux runner (baseline, no impairment)

## Commands

- `cargo test --workspace --all-features`
- `cargo test -p sct-core --test transfer_smoke`
- `cargo test -p sct-core --test prompt5_integration`
- `cargo bench --bench throughput -- --quick`
- `cargo bench --bench latency -- --quick`
- `cargo run -p sct-bench -- --samples 5 --payload-mib 64`

## Summary

- Baseline transport path stable across SSE/QUIC stream/datagram feature sets.
- Prompt-5 sct-core integration suite passed.
- No fallback misfires observed in zero-loss baseline.

## Metrics snapshot

- `fallback_count`: 0
- `events_dropped`: 0
- `events_sent`: >0 across all transport tests
- `sct_synthetic_throughput_mbps`: captured in CI benchmark artifact

## Artifact references

- `datagram-test-results` (experimental workflow artifact)
- `datagram-benchmark-results` (experimental workflow artifact)
