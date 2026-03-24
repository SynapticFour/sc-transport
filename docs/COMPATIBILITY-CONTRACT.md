# Compatibility Contract

This document defines the runtime compatibility surface expected by downstream
consumers such as Synaptic-Core.

## Crate Boundaries

- `sc-transport-core`:
  - `Transport` trait
  - common event/status/error/metrics types
  - stable config-facing contract semantics
- `sc-transport-sse`:
  - stable SSE implementation (`HttpSseTransport`)
- `sc-transport-quic`:
  - QUIC stream implementation (control-plane semantics)
- `sc-transport-datagrams`:
  - experimental best-effort datagram transport (0.x only)

## Semver Policy

- `sc-transport-core`: semver-stable public API; trait and shared types are the
  downstream contract source of truth.
- `sc-transport-sse`: semver-stable implementation crate.
- `sc-transport-quic`: semver-stable optional implementation crate.
- `sc-transport-datagrams`: 0.x experimental; breaking changes may occur.

## Feature Matrix

- `sc-transport-core`: no runtime transport feature flags required.
- `sc-transport-sse`: stable by default.
- `sc-transport-quic`:
  - `quic-streams` enables quinn/rustls/loopback QUIC stream helpers.
- `sc-transport-datagrams`:
  - `quic-datagrams` enables quinn datagram path helpers.

## Toolchain Requirements

- Supported CI baseline: current stable Rust.
- Minimum practical requirement: modern stable Cargo with lockfile v4 support.
- Legacy Cargo versions that do not support lockfile v4/edition2024 transitive
  dependencies are not part of the compatibility target.

## Error and Status Model

- `TransportError`:
  - `Serialization(String)`
  - `Unavailable(String)`
  - `FallbackRequired`
  - `QuicError(String)`
- `DeliveryStatus`:
  - `Sent`
  - `Delivered`
  - `Dropped`
  - `FellBack { reason: String }`

## Metrics Contract

`TransportMetrics` contains:

- `events_sent`
- `events_delivered`
- `events_dropped`
- `fallback_count`
- `current_mode`
- `active_subscribers`

These fields are expected to remain stable for downstream runtime health/status
reporting integrations.
