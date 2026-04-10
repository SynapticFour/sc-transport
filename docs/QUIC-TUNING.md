# QUIC Tuning

## Baseline recommendations

- Set `SO_SNDBUF` and `SO_RCVBUF` to 8 MiB as an initial default.
- Use host networking settings that allow UDP throughput stability.
- Keep kernel and NIC drivers updated on Linux test machines.

## Linux

- Ensure Generic Segmentation Offload (GSO) is available/enabled where possible.
- Validate UDP buffer limits (`net.core.rmem_max`, `net.core.wmem_max`).
- Use `tc netem` only for controlled experiments, not normal benchmarking.

## macOS

- Expect different UDP behavior than Linux due to platform networking stack.
- Prefer loopback and controlled LAN tests for consistency.

## Observability

- Capture sent, delivered, dropped, and fallback counters.
- Record transport mode transitions with timestamps.
- Correlate packet-loss conditions with fallback threshold behavior.

## Pre-transfer path probing

Before initiating a large batch transfer, run `NetworkProbe::measure()` to
classify the link. Feed `NetworkProfile::suggested_config` into `BatchSender`
and `ScientificCongestionController` to pre-configure for your actual path
instead of relying on slow-start discovery.
