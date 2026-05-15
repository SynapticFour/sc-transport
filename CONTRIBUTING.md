# Contributing

## Scope

- Keep `sc-transport-core` and `sc-transport-sse` stable and conservative.
- Keep `sc-transport-datagrams` experimental (0.x) until production-proven.
- SPARQ crates (`sct-core`, `sct-cli`, `sct-daemon`, …) are bootstrap — match existing style and test coverage when changing them.
- Preserve the transparency contract: final workflow state must remain correct
  independent of active transport.

## Development

- Run `make ci` before opening a PR (fmt, clippy, sct-core + transport integration + smoke).
- While iterating: `make test-integration`, `make test-unit`, or targeted `cargo test -p …`.
- See [README § Continuous integration](README.md#continuous-integration) for targets and CI mapping.
- Local benchmark output: `results/` and `crates/sct-core/results/` are gitignored — use `make clean-results` to wipe.

## Documentation-first policy

Before major transport changes, update:

- `docs/QUIC-BACKGROUND.md`, `docs/QUIC-TUNING.md`
- `docs/DATAGRAM-SEMANTICS.md`, `docs/FALLBACK-BEHAVIOR.md`, `docs/LIMITATIONS.md`
- `docs/NETWORK-EMULATION.md` (if CI vs manual netem behavior changes)
- `docs/MIGRATION-SYNAPTIC-CORE.md` (if Synaptic-Core integration contract changes)
