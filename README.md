# sc-transport

Transport layer for [Synaptic Core](https://github.com/SynapticFour/Synaptic-Core).
Provides implementations of the `Transport` trait for telemetry delivery:

**Synaptic Core stack:** see **[docs/ECOSYSTEM.md](docs/ECOSYSTEM.md)** for sc-specs, Synaptic-Core, and Synaptic-Core-Test.

### Local daemon (Docker)

```bash
make up      # start sct-daemon
make down    # stop; keep volumes
make destroy # stop; remove volumes
```

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

# synthetic CPU/memory throughput baseline
cargo run -p sct-bench -- synthetic --samples 5 --payload-mib 64

# Linux only: apply tc/netem good/bad/very-bad matrix and run transfer sizes
make netem-test   # writes docs/RESULTS/<date>-linux-netem-matrix.json

# one-command transfer test wrapper (receiver + sender + checksum)
make transfer-test PROFILE=toronto-auckland SIZE_GB=20
```

For Linux and macOS network emulation playbooks (including a 20 GB Toronto<->Auckland example), see [`docs/NETWORK-EMULATION.md`](docs/NETWORK-EMULATION.md).

## Continuous integration

Primary gate: GitHub Actions [`.github/workflows/ci.yml`](.github/workflows/ci.yml) (two jobs on every push/PR, or **Run workflow** manually).
Additional workflows: `experimental.yml` (datagram benches, non-blocking), `codeql.yml`, `quality-gate.yml`, `secret-scan.yml`, `dependency-review.yml`.

Optional **real-WAN** benchmarks: AWS EC2 workflow [`wan-test.yml`](.github/workflows/wan-test.yml) (Activate Credits, scheduled Mondays) or local Fly.io script [`scripts/wan_test_flyio.sh`](scripts/wan_test_flyio.sh). No cloud account needed for normal CI. See [`docs/WAN-CLOUD-CI.md`](docs/WAN-CLOUD-CI.md).

| Job | Local equivalent | What it runs |
|-----|----------------|--------------|
| **test** | first part of `make ci` | `fmt`, clippy (`sct-core`, `sct-proto`), `sct-core` lib + integration tests (serial QUIC), cf-check p99 gate, `sct-proto` tests |
| **workspace-integration** | `make ci-transport-integration` + `make ci-cli-daemon-smoke` (via `make test-integration` / `make ci`) | Clippy + unit tests for transport crates; `scripts/ci-transport-integration.sh` (`streaming_matrix` smoke + SSE/QUIC/datagram integration); `scripts/ci-cli-daemon-smoke.sh` (`cargo test -p sct-daemon`, `sct` loopback, `sct-daemon` REST receive + push) |

### Make targets

| Target | Use when |
|--------|----------|
| `make ci` | Full pre-push gate (**both** CI jobs: sct-core + transport). |
| `make ci-transport-integration` | Transport workspace only (CI job `workspace-integration`). |
| `make ci-all` | Alias for `make ci`. |
| `make test-integration` | All integration tests, no fmt/clippy/cf-check (~fast feedback while iterating). |
| `make test-integration-sct-core` | sct-core integration binaries only (`RUST_TEST_THREADS=1`). |
| `make test-unit` | Library/unit tests for sct-core + transport crates. |
| `make profile-quic-good-large` | Release bench: `good`/`large` `quic-stream` matrix slice (target ≥400 events/s). |
| `make ci-cli-daemon-smoke` | Subprocess tests: `sct send`/`recv` and `sct-daemon` REST (receive + push). |
| `make netem-test` | Linux+root: full `sct-bench netem-matrix` → dated JSON under `docs/RESULTS/`. |
| `make clean-results` | Remove gitignored local benchmark trees (`results/`, `crates/sct-core/results/`). |

**Notes**

- sct-core integration tests must run **serially** (`--test-threads=1`); the Makefile sets `RUST_TEST_THREADS=1`.
- `e2e_loopback` FEC recovery needs `--features test-hooks` (wired in `make test-integration-sct-core` and CI).
- `streaming_matrix` in CI uses a **smoke** profile (`excellent` × `tiny,small`, 2 repeats). Full matrix locally: see [`docs/NETWORK-EMULATION.md`](docs/NETWORK-EMULATION.md#streaming-only-matrix-no-large-files).
- Transport integration sets `SC_TRANSPORT_ALLOW_INSECURE_QUIC=true`, `SC_QUIC_CONNECT_TIMEOUT_MS=2000`, `SC_QUIC_MIRROR_SSE=1` (see `scripts/ci-transport-integration.sh`).
- CLI/daemon smoke builds `sct` + `sct-daemon`, runs `cargo test -p sct-daemon`, then `cli_smoke` / `daemon_smoke` (receive + push REST paths) with `RUST_TEST_THREADS=1` (see `scripts/ci-cli-daemon-smoke.sh`).
- `sct send --verify` maps to waiting for the receiver `FinalAck` (`require_final_ack`); use `recv --verify` for checksum verification on disk.
- Benchmark JSON/MD from local runs goes under **`results/`** (gitignored); curated summaries belong in [`docs/RESULTS/`](docs/RESULTS/).

```bash
make ci              # full gate (both CI jobs)
make test-integration
make profile-quic-good-large   # optional performance regression check
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

See [`docs/sct-daemon-ops.md`](docs/sct-daemon-ops.md) for the operator runbook. Container:
`docker build -t sct-daemon:latest .` then `docker compose up` (see `Dockerfile`).

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

sc-transport is released under the
[Business Source License 1.1](LICENSE).

**Free for:** research, evaluation, academic use, open-source projects.
**Commercial use:** requires a [commercial license](COMMERCIAL.md).

On 2029-01-01, sc-transport will convert to Apache 2.0.

## Support

Questions or security concerns: [contact@synapticfour.com](mailto:contact@synapticfour.com).

---

Contact: [contact@synapticfour.com](mailto:contact@synapticfour.com)

