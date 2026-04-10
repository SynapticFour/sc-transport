# sc-transport

Transport layer for [Synaptic Core](https://github.com/SynapticFour/Synaptic-Core).
Provides implementations of the `Transport` trait for telemetry delivery:

> **Legal notice:** This repository documents technical capabilities and operating guidance. It is not legal advice and does not by itself provide regulatory certification or compliance guarantees. Compliance outcomes depend on operator configuration, contracts, and organisational controls.

| Crate | Transport | Status | When to use |
|-------|-----------|--------|-------------|
| sc-transport-core | Trait + shared transport contract | **Stable** | Downstream runtime interface and shared types. |
| sc-transport-sse | HTTP SSE (Server-Sent Events) | **Stable** | Default production transport implementation. |
| sc-transport-quic | QUIC reliable streams | **Optional** | When 0-RTT reconnection or no HoL blocking matters. |
| sc-transport-datagrams | QUIC unreliable datagrams | **Experimental (0.x)** | Research only. See [LIMITATIONS](docs/LIMITATIONS.md). |

## SPARQ bootstrap (new)

This repository now also contains a SPARQ bootstrap workspace for high-throughput
scientific data transfer foundations:

| Crate | Purpose | Status |
|-------|---------|--------|
| `sct-proto` | Manifest/chunk wire structures using `serde` + `bincode` | Bootstrap |
| `sct-core` | SPARQ protocol, QUIC endpoint wrapper, sender/receiver MVP | Bootstrap |
| `sct-cli` | `sct send|recv|probe|bench` command surface | Bootstrap |
| `sct-bench` | Benchmark harness entrypoint | Bootstrap |
| `sct-daemon` | REST daemon for queued transfers + recovery | Bootstrap |

Design philosophy:
- Build on open QUIC (IETF RFC 9000) rather than bespoke UDP protocol mechanics.
- Keep control + data semantics explicit and observable in protocol types.
- Start with a compile-safe MVP, then harden with WAN tuning and profiling phases.

Quick CLI examples:

```bash
# receive one transfer
cargo run -p sct-cli -- recv --port 7272 --output-dir /tmp --once

# send with auto compression and json progress
cargo run -p sct-cli -- send ./dataset.bin sct://127.0.0.1:7272 --compression auto --json

# probe and bench
cargo run -p sct-cli -- probe sct://127.0.0.1:7272 --samples 5
cargo run -p sct-cli -- bench sct://127.0.0.1:7272 --samples 5
```

## sct-daemon operations

The daemon provides transfer queueing, priority updates, cancellation, and restart recovery.

- Daemon API: `POST /v1/transfer`, `GET/PATCH/DELETE /v1/transfer/{id}`, `GET /v1/transfers`
- Transfer modes:
  - push: `source=file://...` and `destination=sct://host:port`
  - receive: `source=sct://...` and `destination=<output-dir>`
- Recovery model:
  - snapshot state: `.sct-daemon/transfers.json`
  - append-only event log: `.sct-daemon/events.jsonl`
  - on restart, queued jobs are rebuilt; active jobs are recovered as queued.

Troubleshooting highlights:
- TOFU host-key mismatch: remove stale entries in `~/.sct/known_hosts`.
- Resume leftovers: partial receive files (`*.part`, `*.state.json`) are expected during interrupted transfers and cleaned up on successful completion.

See [`docs/sct-daemon-ops.md`](docs/sct-daemon-ops.md) for the operator runbook.

## Transparency contract

All three transports implement the same `Transport` trait. Switching between
them is a configuration change. Under the documented assumptions and test
scope, clients are expected to observe the same final workflow state regardless
of active transport. If QUIC datagrams underperform, the system falls back to
SSE automatically.

## The experiment

QUIC datagrams for scientific workflow telemetry have not been measured in
the literature. `sc-transport-datagrams` is our attempt to generate that
measurement. See [docs/RESULTS/](docs/RESULTS/) for honest benchmarks as
they are collected.

For final rollout gating, use the
[Production Readiness Checklist](docs/PRODUCTION-READINESS-CHECKLIST.md).

For downstream runtime consumers, see:

- [Compatibility Contract](docs/COMPATIBILITY-CONTRACT.md)
- [Synaptic-Core Migration Guide](docs/MIGRATION-SYNAPTIC-CORE.md)

**`sc-transport-datagrams` will not reach v1.0 until production measurements
confirm it performs as intended in real scientific deployments.**

## License

Apache-2.0. Copyright (c) 2026 Synaptic Four.

## Support

Questions or security concerns: [contact@synapticfour.com](mailto:contact@synapticfour.com).

---

Built by **Synaptic Four** for transparent, standards-based scientific infrastructure.
Developed by a neurodiverse team, including autistic engineers, with a focus on precision, clarity, and reliable operations.
Contact: [contact@synapticfour.com](mailto:contact@synapticfour.com) · [synapticfour.com](https://synapticfour.com)

