# QUIC Background for sc-transport

## 1) Why QUIC for telemetry (not bulk data)?

`sc-transport` targets telemetry/control-plane traffic, not bulk data transfer.
Telemetry streams consist of many small event updates where reconnect speed and
stream independence matter more than absolute throughput.

For this profile, QUIC offers:

- Fast reconnection with session resumption and optional 0-RTT.
- Independent logical streams without TCP head-of-line blocking between streams.
- User-space transport behavior that is easier to instrument experimentally.

Large artifact transfer remains out of scope and should continue to use the
existing HTTP/2 stack.

## 2) What current literature says

- Lychev et al. (WWW 2024) report throughput reductions for QUIC vs TCP at very
  high link rates (for example around 100 Gbit/s) in bulk transfer settings.
  This is not the primary target workload of `sc-transport`.
- ETH Zurich (2024) reports that QUIC implementation choice matters under loss.
  MsQuic outperforms quinn at high loss (>20%), while lower-loss settings can
  be comparable.

Practical implication for `sc-transport`: quinn is appropriate for typical
datacenter and university network conditions, while lossy networks require
aggressive fallback behavior and honest measurement.

## 3) What QUIC datagrams are (RFC 9221)

QUIC datagrams are unreliable and unordered application payloads sent over a
QUIC connection:

- Delivery is not guaranteed.
- Ordering is not guaranteed.
- Congestion control still applies at connection level.

Datagrams are appropriate for fire-and-forget updates where occasional loss is
tolerable. They are not appropriate for required ordered delivery semantics.

## 4) Why try QUIC datagrams for scientific workflow telemetry?

- There is little to no published measurement for this specific workload.
- Telemetry update streams are often bursty and small, which can fit datagram
  usage patterns.
- Potential benefits include lower overhead for long-running workflows.

Risk containment strategy: when datagrams underperform, automatically fall back
to stable SSE behavior so end-user workflow correctness is preserved.

## 5) Known quinn-specific issues

- Socket and segment sizing significantly affect practical throughput.
- Linux Generic Segmentation Offload (GSO) can materially affect performance.
- Behavior differs across OS/network combinations and must be measured.

See `docs/QUIC-TUNING.md` for operational knobs and defaults.
