# Benchmark I-final — 2026-05-16

## Setup
- Commit: `8c5d8e3f8260090da2c5f503bd3b175ad0d1709a`
- Release build, repeats=5, loopback (127.0.0.1), RUST_TEST_THREADS=1
- Datagram-Extension: aktiv (datagram_receive_buffer_size=1MiB client+server)

## Highlights vs G (Ausgangspunkt, 2026-03-23)

| Szenario | G | I-final | Δ |
|---|---|---|---|
| quic-stream/excellent/large | 13 094 eps | 17 237 eps | +31.6% |
| quic-stream/excellent/small | 22 241 eps | 25 113 eps | +12.9% |
| sse/excellent/large | 10 833 eps | 72 639 eps | +570%* |
| quic-datagram/excellent/tiny (fallback) | 2/200 | 2/200 | — |
| quic-datagram/excellent/small (fallback) | 200/200 | 200/200 | — (4096 B > 1200 B MTU) |
| cf-check p99 | 31 ms | 0.015 ms | −99.95% |

\*Matrix sendet seit H/I mehr Events pro Fall (300/400/500/600 vs. 100 in G); eps sind nicht 1:1 vergleichbar ohne Normalisierung.

## Vollständige Daten
Siehe `results/I-final/streaming-matrix-summary.json` und
`results/I-final/competitive-baseline-matrix.md`.

Gegenüberstellung: `results/streaming-matrix-compare-G-vs-I-final.md`.

## Bekannte Einschränkungen
- quic-datagram/large immer Fallback (262 KB > MTU 1200 B — by design)
- quic-datagram/excellent/small: Payload 4096 B > `DGRAM_SAFE_MTU` (1200 B) → SSE-Fallback erwartet
- poor/very-poor eps auf ~120/67 eps durch WAN-Simulation (loopback netem)
- Echte WAN-Messung (EC2 Toronto↔Frankfurt) steht noch aus
