# Fallback Behavior

Fallback protects correctness over transport preference.

## Trigger classes

- QUIC endpoint setup failure at startup.
- Per-connection packet loss above configured threshold.
- Peer capability mismatch (for example no datagram support).
- Explicit forced fallback signals in experimental tests.

## Mode transitions

- QUIC stream/datagram mode can transition to SSE.
- Fallback is per-connection for datagrams.
- Existing healthy connections may remain on QUIC.

## Signaling

- Emit a `TransportFallback` telemetry event when fallback occurs.
- Increment `fallback_count` in transport metrics.
- Log fallback reason and observed conditions.

## Experimental fallback controller

- Datagram transport tracks a rolling delivery/loss window.
- Loss above configured threshold triggers `DeliveryStatus::FellBack`.
- Fallback decisions are isolated to the affected transport path.
