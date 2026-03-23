# LIMITATIONS (Experimental Datagram Transport)

This document applies to `sc-transport-datagrams` (v0.x only).

## Status

- Experimental research implementation.
- Not production-proven.
- API and behavior may evolve before 1.0.

## Delivery semantics

- Datagram delivery is unreliable and unordered by design.
- Loss, reordering, and occasional burst drop behavior should be expected.
- `DeliveryStatus::Sent` does not imply end-to-end receive.

## Network sensitivity

- Delivery quality is highly sensitive to packet loss and path quality.
- Performance is environment dependent and cannot be inferred from one setup.
- Loss-heavy paths may trigger automatic fallback to SSE.

## MTU and payload constraints

- Maximum payload must honor connection-specific datagram size limits.
- Oversized events may be truncated, summarized, or dropped.
- Operators must treat payload size discipline as part of transport correctness.

## Rate limiting effects

- Per-run send limits intentionally drop lower-priority updates under pressure.
- Progress-type events may be dropped before state-transition events.
- Final state correctness must be validated by higher-layer state logic.

## Fallback behavior

- Fallback decisions are per-connection, not global.
- Fallback protects correctness at the cost of transport-mode consistency.
- Under persistent degradation, SSE should be considered expected behavior.

## Measurement policy

- Report all measured scenarios, including unfavorable outcomes.
- Do not cherry-pick best-case benchmarks.
- Keep artifacts under `docs/RESULTS/` with reproducible conditions.
