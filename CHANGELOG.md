# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- `sc-transport-quic`: QUIC Datagram-Extension (RFC 9221) war nicht ausgehandelt
  weil `TransportConfig.datagram_receive_buffer_size` fehlte. Alle Datagram-Sends
  fielen auf SSE-Fallback zurück (200/200 Ereignisse). Behoben durch explizites
  Setzen von 1-MiB-Puffern in `connect_for_batch`, `sc-transport-datagrams`
  Client-Connect und Testinfrastruktur.

### Added

- `.github/workflows/ci.yml` und `quality-gate.yml`: automatische CI auf
  push/PR. Schützt gegen Rückfälle bei completion_first, parity_shards,
  und anderen mehrfach regredierten Fixes.
- `docs/RESULTS/2026-05-16-benchmark-I-final.md`: vollständige Streaming-Matrix
  nach allen Korrekturen seit G (2026-03-23).

### Changed

- Benchmark I-final (release, repeats=5): quic-stream/excellent/large 17 237 eps
  (+32% vs G). cf-check p99 = 0.015 ms (war 31 ms). Datagram-Extension aktiv;
  excellent/tiny fallback 2/200 (unverändert vs. G). excellent/small weiterhin
  200/200 Fallback wegen 4096 B > 1200 B MTU-Policy.

- Introduced `sc-transport-sse` as dedicated stable SSE runtime crate.
- Added compatibility contract and Synaptic-Core migration documentation.
- Added deterministic fixture-driven conformance behavior checks.
- Added optional CI artifact emission from transport parity tests.

### Changed

- `sc-transport-core` is now contract-only (trait/types/metrics/error surface).
- Root feature flags now expose downstream-friendly runtime toggles:
  - `transport-sse`
  - `transport-quic`
  - `transport-datagrams`

### Synaptic-Core Integration Impact

- Downstream runtime can replace internal transport wiring with released
  `sc-transport-*` crates using mechanical type/feature mapping.
- `v0.1.0` compatibility for existing downstream pinning is retained while
  migration guidance is now explicit and test-backed.
