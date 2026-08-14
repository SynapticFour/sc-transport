# QUIC Tuning

## Baseline recommendations

- Set `SO_SNDBUF` and `SO_RCVBUF` to 8 MiB as an initial default.
- Use host networking settings that allow UDP throughput stability.
- Keep kernel and NIC drivers updated on Linux test machines.

## Linux

- Ensure Generic Segmentation Offload (GSO) is available/enabled where possible.
  SPARQ does not reimplement segmentation: Quinn/quinn-udp uses kernel GSO/GRO on Linux.
  `TransportConfig.enable_gso` is an operator hint in logs, not a second offload protocol.
- Validate UDP buffer limits (`net.core.rmem_max`, `net.core.wmem_max`).
- Use `tc netem` only for controlled experiments, not normal benchmarking.

## macOS

- Expect different UDP behavior than Linux due to platform networking stack.
- Prefer loopback and controlled LAN tests for consistency.
- Use `pf` + `dummynet` (`pfctl` + `dnctl`) as the macOS equivalent of Linux `tc netem`.
- See [`docs/NETWORK-EMULATION.md`](NETWORK-EMULATION.md) for command-ready profiles and cleanup.

## QUIC stream throughput (matrix / good + large)

Target for competitive baseline: **≥400 events/s** on `good` / `large` (262 KiB payloads, loopback).

| Knob | Default | Effect |
|------|---------|--------|
| `SC_QUIC_PERSISTENT_UNI` | `true` | Reuse one uni stream per connection instead of `open_uni` per event |
| `SC_QUIC_MIRROR_SSE` | `false` | Duplicate each QUIC send into in-process SSE (needed only for subscribe tests) |
| `SC_QUIC_STREAM_FLUSH_EVERY` | `16` | Flush persistent uni stream every N writes |
| `SC_QUIC_COALESCE_EVENTS` | off | Batch N progress events into one framed payload before send |

Profile locally:

```bash
make profile-quic-good-large
```

## Observability

- Capture sent, delivered, dropped, and fallback counters.
- Record transport mode transitions with timestamps.
- Correlate packet-loss conditions with fallback threshold behavior.

## Pre-transfer path probing

SPARQ CLI `sct probe` uses `CongestionOracle` (QUIC RTT/cwnd samples on a SPARQ
endpoint). The telemetry crate `sc-transport-probe::NetworkProbe` is a separate
path profiler for event transports; it is not wired into `sct probe`.

Before a large telemetry batch, `NetworkProbe::measure()` can classify a link
and feed `NetworkProfile::suggested_config` into `BatchSender`. SPARQ file
transfer instead samples Quinn RTT/cwnd into Scientific BBR + the hybrid
scheduler on every send batch.
