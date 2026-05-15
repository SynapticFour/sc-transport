# Local benchmark output (not in git)

This directory is **gitignored**. Integration tests and helper scripts write here when
`SC_TRANSPORT_ARTIFACT_DIR` is set (default paths in `scripts/*.py` and
`docs/NETWORK-EMULATION.md`).

## What lands here

| Pattern | Source |
|---------|--------|
| `streaming-matrix*.json` | `streaming_matrix` integration test |
| `competitive-baseline-matrix.*` | `scripts/competitive_baseline_matrix.py` |
| `completion-campaign-*` | `scripts/compare_completion_campaign.py`, cf-check runs |
| `cf-check/` | `make ci` writes `results/cf-check/` (completion_first p99 gate) |
| `cf-*/`, `A/`, `B/`, … | Archived experiment dirs from local matrix runs |

## Versioned snapshots

Commit **human-readable summaries** under [`docs/RESULTS/`](../docs/RESULTS/) using
[`docs/RESULTS/template.md`](../docs/RESULTS/template.md). Do not commit raw JSON from
this folder unless explicitly agreed for a one-off baseline.

## Quick start

```bash
mkdir -p results
SC_TRANSPORT_ARTIFACT_DIR=results \
  cargo test -p sc-transport --features transport-quic,transport-datagrams \
  --test streaming_matrix -- --exact
```

**CI:** GitHub Actions uses `/tmp` for cf-check and does not populate this directory.
**Local `make ci`:** intentionally writes `results/cf-check/` for the completion_first gate.

```bash
make clean-results   # remove gitignored subtrees (keeps this README)
```
