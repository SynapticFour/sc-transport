# Measurement Record

## WAN Simulation Results

| Scenario | Transport | Events/sec | Loss Rate | Fallback Count | RTT | Notes |
|----------|-----------|------------|-----------|----------------|-----|-------|
| continental | quic-stream | — | — | — | 50ms | |
| intercontinental | datagram | — | — | — | 100ms | |
| degraded | datagram | — | — | — | 150ms | |

Fill in values after running: `./scripts/netem_runner.sh run_wan_scenarios`
