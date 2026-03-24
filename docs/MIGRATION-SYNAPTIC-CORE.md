# Migration Guide: Synaptic-Core Runtime

This guide covers replacing internal runtime transport wiring with released
`sc-transport-*` crates.

## 1) Dependency Migration

Replace internal transport runtime dependency with:

- `sc-transport-core`
- `sc-transport-sse`
- optionally `sc-transport-quic`
- optionally `sc-transport-datagrams`

## 2) Type/Function Mapping

| Old (internal) | New (`sc-transport-*`) |
|---|---|
| `synaptic_core_transport::Transport` | `sc_transport_core::Transport` |
| `synaptic_core_transport::TelemetryEvent` | `sc_transport_core::TelemetryEvent` |
| internal SSE runtime impl | `sc_transport_sse::HttpSseTransport` |
| internal QUIC stream impl | `sc_transport_quic::QuicStreamTransport` |
| internal datagram experiment | `sc_transport_datagrams::QuicDatagramTransport` |

## 3) Feature Flag Mapping

| Old feature | New feature |
|---|---|
| internal quic stream flag | `sc-transport-quic/quic-streams` |
| internal datagram flag | `sc-transport-datagrams/quic-datagrams` |

## 4) Runtime Startup Snippet (Axum/Tokio)

```rust
use std::sync::Arc;
use sc_transport_core::Transport;
use sc_transport_sse::HttpSseTransport;
use sc_transport_quic::QuicStreamTransport;
use sc_transport_datagrams::QuicDatagramTransport;

fn build_transport() -> Arc<dyn Transport> {
    // Replace selection logic with your own feature/config policy.
    #[cfg(feature = "use_datagrams")]
    {
        return Arc::new(QuicDatagramTransport::new());
    }
    #[cfg(all(not(feature = "use_datagrams"), feature = "use_quic_streams"))]
    {
        return Arc::new(QuicStreamTransport::new());
    }
    Arc::new(HttpSseTransport::new())
}
```

## 5) Validation Checklist

- Run transport conformance tests in `sc-transport`.
- Verify final state transparency test passes.
- Verify runtime wiring in Synaptic-Core uses released tags, not local patches.
- Confirm `TransportMetrics` surface is consumed unchanged by health endpoints.
