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
| `cf-*/` | Archived `completion-campaign-summary.json` from A/B runs |
| `A/`, `B/`, … | Closed-loop matrix experiment dirs |

## Versioned snapshots

Commit **human-readable summaries** under [`docs/RESULTS/`](../docs/RESULTS/) using
[`docs/RESULTS/template.md`](../docs/RESULTS/template.md). Do not commit raw JSON from
this folder unless explicitly agreed for a one-off baseline (prefer markdown + inputs in
`docs/RESULTS`).

## Quick start

```bash
mkdir -p results
SC_TRANSPORT_ARTIFACT_DIR=results \
  cargo test -p sc-transport --features transport-quic,transport-datagrams \
  --test streaming_matrix -- --exact
```

CI uses `/tmp` or job-local paths instead of this directory.
