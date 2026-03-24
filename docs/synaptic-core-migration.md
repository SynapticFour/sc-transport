# Synaptic-Core Migration Guide

Use this guide to replace Synaptic-Core runtime usage of
`synaptic-core-transport` with `sc-transport-*`.

## Internal -> External Mapping

| Internal (old) | sc-transport (new) |
|---|---|
| `synaptic_core_transport::Transport` | `sc_transport_core::Transport` |
| `synaptic_core_transport::TelemetryEvent` | `sc_transport_core::TelemetryEvent` |
| `synaptic_core_transport::TransportMetrics` | `sc_transport_core::TransportMetrics` |
| internal SSE implementation | `sc_transport_sse::HttpSseTransport` |
| internal QUIC stream implementation | `sc_transport_quic::QuicStreamTransport` |
| internal datagram implementation | `sc_transport_datagrams::QuicDatagramTransport` |

## Feature Flag Mapping

| Downstream flag | sc-transport feature |
|---|---|
| `transport-sse` | `sc-transport/transport-sse` |
| `transport-quic` | `sc-transport/transport-quic` |
| `transport-datagrams` | `sc-transport/transport-datagrams` |

`transport-sse` is default; QUIC and datagrams are additive/optional.

## Axum/Tokio Startup Example

```rust
use std::sync::Arc;
use sc_transport_core::Transport;
use sc_transport_datagrams::QuicDatagramTransport;
use sc_transport_quic::QuicStreamTransport;
use sc_transport_sse::HttpSseTransport;

pub fn build_transport(mode: &str) -> Arc<dyn Transport> {
    match mode {
        "quic-datagram" => Arc::new(QuicDatagramTransport::new()),
        "quic-stream" => Arc::new(QuicStreamTransport::new()),
        _ => Arc::new(HttpSseTransport::new()),
    }
}
```

## Fallback Expectations

- SSE is the stable baseline and always-valid fallback target.
- QUIC stream paths should preserve reliable delivery semantics.
- Datagram paths are best-effort and may drop events.
- On fallback trigger, downstream should expect:
  - `DeliveryStatus::FellBack { reason }`
  - updated `TransportMetrics.fallback_count`
  - final workflow state correctness preserved.

## Mechanical Replacement Checklist

1. Replace internal transport dependency imports with `sc_transport_*`.
2. Switch runtime factory to construct a `dyn sc_transport_core::Transport`.
3. Map existing feature flags to `transport-sse`/`transport-quic`/`transport-datagrams`.
4. Keep health endpoint wiring against `TransportMetrics`.
5. Run conformance tests and consume generated artifacts in downstream CI.
