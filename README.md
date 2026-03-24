# sc-transport

Transport layer for [Synaptic Core](https://github.com/SynapticFour/synaptic-core).
Provides implementations of the `Transport` trait for telemetry delivery:

| Crate | Transport | Status | When to use |
|-------|-----------|--------|-------------|
| sc-transport-core | Trait + shared transport contract | **Stable** | Downstream runtime interface and shared types. |
| sc-transport-sse | HTTP SSE (Server-Sent Events) | **Stable** | Default production transport implementation. |
| sc-transport-quic | QUIC reliable streams | **Optional** | When 0-RTT reconnection or no HoL blocking matters. |
| sc-transport-datagrams | QUIC unreliable datagrams | **Experimental (0.x)** | Research only. See [LIMITATIONS](docs/LIMITATIONS.md). |

## The transparency guarantee

All three transports implement the same `Transport` trait. Switching between
them is a configuration change. Clients receive identical final workflow state
regardless of which transport is active. If QUIC datagrams underperform, the
system falls back to SSE automatically.

## The experiment

QUIC datagrams for scientific workflow telemetry have not been measured in
the literature. `sc-transport-datagrams` is our attempt to generate that
measurement. See [docs/RESULTS/](docs/RESULTS/) for honest benchmarks as
they are collected.

For final rollout gating, use the
[Production Readiness Checklist](docs/PRODUCTION-READINESS-CHECKLIST.md).

For downstream runtime consumers, see:

- [Compatibility Contract](docs/COMPATIBILITY-CONTRACT.md)
- [Synaptic-Core Migration Guide](docs/MIGRATION-SYNAPTIC-CORE.md)

**`sc-transport-datagrams` will not reach v1.0 until production measurements
confirm it performs as intended in real scientific deployments.**

## License

Apache-2.0. Copyright (c) 2026 Synaptic Four.
