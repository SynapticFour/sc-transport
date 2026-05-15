# Contributing

## Scope

- Keep `sc-transport-core` stable and conservative.
- Keep `sc-transport-datagrams` experimental (0.x) until production-proven.
- Preserve the transparency contract: final workflow state must remain correct
  independent of active transport.

## Development

- Run `make ci-all` before opening a PR (or `make ci` + `make ci-transport-integration` separately).
- While iterating: `make test-integration` (integration only) or `make test-unit`.
- See [README § Continuous integration](README.md#continuous-integration) for the full target matrix and CI job mapping.

## Documentation-first policy

Before major transport changes, update:

- `docs/QUIC-BACKGROUND.md`
- `docs/QUIC-TUNING.md`
- `docs/DATAGRAM-SEMANTICS.md`
- `docs/FALLBACK-BEHAVIOR.md`
- `docs/LIMITATIONS.md` (for datagram behavior changes)
