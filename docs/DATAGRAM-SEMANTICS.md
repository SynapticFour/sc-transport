# Datagram Semantics

`sc-transport-datagrams` provides best-effort event delivery.

## Guarantees

- No guaranteed delivery.
- No guaranteed ordering.
- No guaranteed exactly-once behavior.

## Practical interpretation

- Treat datagram events as opportunistic updates.
- Preserve state-transition events wherever possible.
- Derive final workflow state from durable state logic, not transient event count.
- Oversized payloads are summarized/truncated to fit datagram size limits.
- Progress events may be dropped first when the test hook
  `SC_DGRAM_PROGRESS_DROP_EVERY` is set (default **0**, disabled). This is not a
  production rate limiter.

## Compatibility with transparency contract

- Event sets may differ under datagrams.
- Final derived workflow state must remain equivalent across transports.
- Automatic fallback to SSE is the safety mechanism when network conditions are poor.

## Current implementation notes

- Datagram payloads are MessagePack-encoded `TelemetryEvent` values.
- A rolling loss window is maintained to estimate current loss behavior.
- If estimated loss exceeds threshold (default 15%), transport emits fallback status.
- A feature-gated QUIC datagram loopback path exists for protocol-level roundtrip tests.
