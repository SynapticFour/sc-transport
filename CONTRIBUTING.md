# Contributing

## Scope

- Keep `sc-transport-core` stable and conservative.
- Keep `sc-transport-datagrams` experimental (0.x) until production-proven.
- Preserve the transparency contract: final workflow state must remain correct
  independent of active transport.

## Development

- Run `cargo fmt --all`.
- Run `cargo clippy --workspace --all-features -- -D warnings`.
- Run `cargo test --workspace`.

## Documentation-first policy

Before major transport changes, update:

- `docs/QUIC-BACKGROUND.md`
- `docs/QUIC-TUNING.md`
- `docs/DATAGRAM-SEMANTICS.md`
- `docs/FALLBACK-BEHAVIOR.md`
- `docs/LIMITATIONS.md` (for datagram behavior changes)
