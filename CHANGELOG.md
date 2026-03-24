# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
