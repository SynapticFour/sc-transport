# Linux Netem 20% Loss Report

Date: 2026-04-10  
Scope: `sc-transport` prompt 1-5 production-hard closeout  
Impairment: `tc netem loss 20%` on loopback during fallback validation

## Commands

- `scripts/netem_runner.sh lo 20 datagram_fallback_trigger`
- `scripts/netem_runner.sh lo 20 datagram_delivery_20pct_loss`
- `cargo test --features "transport-quic transport-datagrams" --test transport_transparency`

## Summary

- Fallback trigger behavior validated at high loss.
- Final workflow state remained contract-compatible after fallback.
- No blocker regression observed for primary SSE/QUIC stream operation.

## Metrics snapshot

- `fallback_count`: non-zero and expected
- `events_dropped`: elevated in datagram mode under 20% loss (expected for experimental path)
- `events_sent`: maintained after fallback

## Conclusion

- High-loss scenarios do not block primary production-hard sign-off.
- Datagram path remains experimental and explicitly non-blocking for primary release.
