# Architecture

This repository uses a modular structure with clear ownership boundaries and explicit integration points.

## Reference map

- `README.md`: project scope and usage entrypoint.
- `docs/`: detailed design notes, operational guidance, and implementation decisions.
- `DECISIONS.md`: architectural decisions and rationale.
- `TESTING.md` (where present): verification strategy and quality gates.

## Design principles

- Keep interfaces stable and versioned.
- Prefer explicit contracts over implicit behavior.
- Treat security, observability, and operability as first-class concerns.
- Document trade-offs and migration paths before introducing breaking changes.
