# Architecture

This repository uses a modular structure with clear ownership boundaries and explicit integration points.

## Reference map

- `README.md`: project scope and usage entrypoint.
- `docs/`: detailed design notes, operational guidance, and implementation decisions.
- `DECISIONS.md`: repo-wide ADR-lite; transport decisions in `docs/DECISIONS.md`.
- [README § Continuous integration](README.md#continuous-integration): verification gates and Make targets.

## Design principles

- Keep interfaces stable and versioned.
- Prefer explicit contracts over implicit behavior.
- Treat security, observability, and operability as first-class concerns.
- Document trade-offs and migration paths before introducing breaking changes.
