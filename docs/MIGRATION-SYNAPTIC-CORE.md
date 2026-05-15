# Migration Guide: Synaptic-Core Runtime

This guide covers replacing internal runtime transport wiring with released
`sc-transport-*` crates.

**Synaptic-Core status (2026-05):** Core currently pins `sc-transport-core` at git tag
`v0.1.1` for `TransportMetrics` types only. The `Transport` trait and SSE/QUIC
implementations are not wired into the live telemetry path yet. Internal QUIC
(`quic-control`) remains separate from `sc-transport-quic` (`quic-stream`). Track
progress in Synaptic-Core [`SC-TRANSPORT-INTEGRATION-CHECKLIST`](https://github.com/SynapticFour/Synaptic-Core/blob/main/docs/SC-TRANSPORT-INTEGRATION-CHECKLIST.md).

## 1) Dependency Migration

Replace internal transport runtime dependency with:

- `sc-transport-core` (required)
- `sc-transport-sse` (stable default)
- optionally `sc-transport-quic` (`quic-streams` feature)
- optionally `sc-transport-datagrams` (`quic-datagrams` feature)

Pin a **git tag** that matches the `sc-transport` release you validated (bump when
`main` advances beyond the pinned commit).

## 2) Type/Function Mapping

| Old (internal) | New (`sc-transport-*`) |
|---|---|
| `synaptic_core_transport::Transport` | `sc_transport_core::Transport` |
| `synaptic_core_transport::TelemetryEvent` | `sc_transport_core::TelemetryEvent` |
| `synaptic_core_transport::TransportMetrics` | `sc_transport_core::TransportMetrics` |
| internal SSE runtime impl | `sc_transport_sse::HttpSseTransport` |
| internal QUIC stream impl | `sc_transport_quic::QuicStreamTransport` |
| internal datagram experiment | `sc_transport_datagrams::QuicDatagramTransport` |

## 3) Feature Flag Mapping

| Downstream flag | sc-transport feature |
|---|---|
| `transport-sse` | `sc-transport/transport-sse` |
| `transport-quic` | `sc-transport-quic/quic-streams` |
| `transport-datagrams` | `sc-transport-datagrams/quic-datagrams` |

`transport-sse` is the stable baseline; QUIC and datagrams are additive/optional.

## 4) Runtime Startup Snippet (Axum/Tokio)

```rust
use std::sync::Arc;
use sc_transport_core::Transport;
use sc_transport_sse::HttpSseTransport;
use sc_transport_quic::QuicStreamTransport;
use sc_transport_datagrams::QuicDatagramTransport;

pub fn build_transport(mode: &str) -> Arc<dyn Transport> {
    match mode {
        "quic-datagram" => Arc::new(QuicDatagramTransport::new()),
        "quic-stream" => Arc::new(QuicStreamTransport::new()),
        _ => Arc::new(HttpSseTransport::new()),
    }
}
```

Wire `Transport::metrics()` into health endpoints instead of `TransportMetrics::default()`.

Implement the AsyncAPI route `GET /sc/transport/v1/runs/{id}/events` using
`HttpSseTransport::subscribe` (see `sc-specs/specs/sc-transport/v1/asyncapi.yaml`).

## 5) Fallback Expectations

- SSE is the stable baseline and always-valid fallback target.
- QUIC stream paths should preserve reliable delivery semantics.
- Datagram paths are best-effort and may drop events.
- On fallback trigger, downstream should expect:
  - `DeliveryStatus::FellBack { reason }`
  - updated `TransportMetrics.fallback_count`
  - final workflow state correctness preserved.

## 6) Validation Checklist

- Run transport conformance tests in `sc-transport` (`make ci` or `make test-integration`).
- Verify `transport_transparency` and integration tests pass.
- Verify runtime wiring in Synaptic-Core uses a **released tag**, not an stale pin.
- Confirm `TransportMetrics` surface is consumed from a live `Transport` instance.
- Run Synaptic-Core-Test `TRANSPORT-*` suite after SSE route is mounted.
