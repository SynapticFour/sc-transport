# Results Archive

This directory stores **versioned** benchmark and integration measurement records for
`sc-transport`, including negative or inconclusive outcomes.

Use `template.md` for new entries and keep measurement inputs reproducible.

## vs `results/` at repo root

| Location | In git? | Purpose |
|----------|---------|---------|
| **`docs/RESULTS/`** (here) | Yes | Curated markdown snapshots, decisions, reproducible commands |
| **`results/`** | No (gitignored) | Local JSON/MD from tests and `scripts/*.py`; safe to delete anytime |

After a local run, copy the conclusions (not every JSON) into a dated file here, e.g.
`2026-05-15-quic-stream-good-large.md`.
