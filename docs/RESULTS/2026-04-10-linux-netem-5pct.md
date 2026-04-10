# Linux Netem 5% Loss Report

Date: 2026-04-10  
Scope: `sc-transport` prompt 1-5 production-hard closeout  
Impairment: `tc netem loss 5%` on loopback during targeted tests

## Commands

- `scripts/netem_runner.sh lo 5 datagram_delivery_5pct_loss`
- `cargo test --features transport-quic --test quic_stream_basic`
- `cargo test --features "transport-quic transport-datagrams" --test transport_transparency`

## Summary

- Delivery under moderate loss remains functional.
- Transparency contract stayed intact for final workflow state.
- No unrecovered transfer failures in sct-core resume stress suite.

## Metrics snapshot

- `fallback_count`: increased under impairment, expected behavior
- `events_dropped`: bounded and within expected experimental datagram tolerance
- `events_sent`: sustained

## Notes

- Datagram behavior remains experimental by product policy.
- Primary readiness sign-off is based on SSE/QUIC stream path.
