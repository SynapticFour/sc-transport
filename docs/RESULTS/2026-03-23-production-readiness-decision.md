# Production Readiness Decision Record

Date: 2026-03-23  
Project: `sc-transport`  
Scope evaluated: Initial repository release candidate

## Section Status (A-D)

| Section | Status | Notes |
|---|---|---|
| A) Linux Netem Measurement Campaign | **Pending** | Local and simulated checks pass; Linux root/netem campaign still required for final sign-off. |
| B) QUIC Primary Path Validation | **Partial** | QUIC stream/datagram loopback and framing paths validated; full production wiring still to be validated in deployment context. |
| C) Transparency Contract Sign-off | **Pass (Current Scope)** | Integration tests confirm final state equivalence across SSE/QUIC/datagram in current implementation. |
| D) Operational Readiness | **Pass (Current Scope)** | CI, experimental workflow, benchmark compare, and issue-creation path are in place. |

## Evidence Snapshot

- Workspace checks:
  - `cargo clippy --workspace --all-features -- -D warnings` passed
  - `cargo test --workspace --all-features` passed
- Bench harness:
  - Throughput/latency/packet-loss benches compile and run in quick mode
- Documentation:
  - QUIC background, tuning, datagram semantics, fallback behavior, limitations, and results template are present
- CI:
  - Stable CI and non-gating experimental CI configured

## Known Risks Accepted (Current Stage)

- Linux `tc netem` measurement campaign not yet completed.
- QUIC behavior may differ under real packet loss and OS/network variability.
- Experimental datagram crate remains 0.x and is not production-proven.

## Decision

Current decision: **Conditional Go (pre-production / research release)**.

This repository state is suitable for continued integration, internal validation,
and controlled experimental use. Final production go-live requires section A to
be completed and section B to be fully validated in deployment-like conditions.

## Approvals

- Engineering Lead: ____________________
- Platform Lead: _______________________
- Release Manager: _____________________
