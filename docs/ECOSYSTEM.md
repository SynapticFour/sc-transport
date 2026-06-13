# SynapticFour Synaptic Core stack

Four repositories implement a domain-agnostic scientific compute fabric. This file is **mirrored** in each repo so readers can navigate without relearning structure.

**You are here:** [sc-transport](https://github.com/SynapticFour/sc-transport) — telemetry (SSE/QUIC) and SPARQ high-throughput transfer (`sct-cli`, `sct-daemon`).

## Repositories

| Repository | Role | License |
|------------|------|---------|
| [sc-specs](https://github.com/SynapticFour/sc-specs) | OpenAPI / AsyncAPI specifications | CC0-1.0 |
| [Synaptic-Core](https://github.com/SynapticFour/Synaptic-Core) | Reference Rust server | BUSL-1.1 |
| **sc-transport** | Transport + SPARQ (this repo) | BUSL-1.1 |
| [Synaptic-Core-Test](https://github.com/SynapticFour/Synaptic-Core-Test) | Conformance runner | Apache-2.0 |

## Local lifecycle (unified commands)

| Repository | Deploy | Stop | Destroy | Notes |
|------------|--------|------|---------|-------|
| **Synaptic-Core** | `make up` | `make down` | `make destroy` | Postgres + MinIO + server |
| **sc-transport** | `make up` | `make down` | `make destroy` | `sct-daemon` via compose |
| **sc-specs** | — | — | — | `make validate` |
| **Synaptic-Core-Test** | — | — | — | Conformance runner |

**SPARQ daemon (this repo):**

```bash
make up
cargo run -p sct-cli -- recv --port 7272 --output-dir /tmp/sct-recv --once
make down
```

CI and benchmarks: `make ci`, `make transfer-test` — see README.

## Documentation map

| Topic | Document |
|-------|----------|
| Transport limits | [docs/LIMITATIONS.md](docs/LIMITATIONS.md) |
| Network emulation | [docs/NETWORK-EMULATION.md](docs/NETWORK-EMULATION.md) |
| Synaptic Core integration | [Synaptic-Core docs/TRANSPORT.md](https://github.com/SynapticFour/Synaptic-Core/blob/main/docs/TRANSPORT.md) |
