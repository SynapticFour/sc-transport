#!/usr/bin/env bash
# Full benchmark matrix re-run (streaming 4×4×3, competitive baseline, compares, cf-check).
set -euo pipefail
set -o pipefail
cd "$(dirname "$0")/.."

RUN_ID="${RUN_ID:-rerun-$(date +%F)}"
export SC_TRANSPORT_ARTIFACT_DIR="results/${RUN_ID}"
export SC_STREAM_MATRIX_REPEATS="${SC_STREAM_MATRIX_REPEATS:-5}"
export SC_TRANSPORT_ALLOW_INSECURE_QUIC="${SC_TRANSPORT_ALLOW_INSECURE_QUIC:-true}"
export SC_QUIC_MIRROR_SSE="${SC_QUIC_MIRROR_SSE:-1}"
export RUST_TEST_THREADS=1

mkdir -p "${SC_TRANSPORT_ARTIFACT_DIR}"
LOG="${SC_TRANSPORT_ARTIFACT_DIR}/run.log"

{
  echo "commit=$(git rev-parse HEAD)"
  echo "started=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "artifact_dir=${SC_TRANSPORT_ARTIFACT_DIR}"
  echo "repeats=${SC_STREAM_MATRIX_REPEATS}"
} | tee "${LOG}"

echo "=== streaming matrix (release) ===" | tee -a "${LOG}"
cargo test --release -p sc-transport --features transport-quic,transport-datagrams \
  --test streaming_matrix -- --nocapture 2>&1 \
  | tee "${SC_TRANSPORT_ARTIFACT_DIR}/streaming-matrix-raw.txt"
test "${PIPESTATUS[0]}" -eq 0

echo "=== competitive baseline ===" | tee -a "${LOG}"
python3 scripts/competitive_baseline_matrix.py \
  --sc-summary "${SC_TRANSPORT_ARTIFACT_DIR}/streaming-matrix-summary.json" \
  --repeats "${SC_STREAM_MATRIX_REPEATS}" \
  --out-json "${SC_TRANSPORT_ARTIFACT_DIR}/competitive-baseline-matrix.json" \
  --out-md "${SC_TRANSPORT_ARTIFACT_DIR}/competitive-baseline-matrix.md"

if [[ -f results/streaming-matrix-summary-G.json ]]; then
  echo "=== compare vs G ===" | tee -a "${LOG}"
  python3 scripts/compare_streaming_matrix.py \
    --before results/streaming-matrix-summary-G.json \
    --after "${SC_TRANSPORT_ARTIFACT_DIR}/streaming-matrix-summary.json" \
    --out-md "results/streaming-matrix-compare-G-vs-${RUN_ID}.md" \
    --out-json "results/streaming-matrix-compare-G-vs-${RUN_ID}.json"
fi

if [[ -f results/I-final/streaming-matrix-summary.json ]]; then
  echo "=== compare vs I-final ===" | tee -a "${LOG}"
  python3 scripts/compare_streaming_matrix.py \
    --before results/I-final/streaming-matrix-summary.json \
    --after "${SC_TRANSPORT_ARTIFACT_DIR}/streaming-matrix-summary.json" \
    --out-md "results/streaming-matrix-compare-I-final-vs-${RUN_ID}.md" \
    --out-json "results/streaming-matrix-compare-I-final-vs-${RUN_ID}.json"
fi

echo "=== completion-first campaign (cf-check) ===" | tee -a "${LOG}"
SC_TRANSPORT_ARTIFACT_DIR="${SC_TRANSPORT_ARTIFACT_DIR}/cf-check" \
  cargo test --release -p sct-core --test completion_campaign completion_campaign_metrics \
  -- --exact --nocapture 2>&1 | tee -a "${LOG}"

echo "=== sct-bench synthetic ===" | tee -a "${LOG}"
cargo run --release -p sct-bench -- synthetic --samples 5 --payload-mib 64 2>&1 \
  | tee "${SC_TRANSPORT_ARTIFACT_DIR}/synthetic-bench.txt"

if [[ "$(uname -s)" == "Linux" ]] && [[ "$(id -u)" -eq 0 ]]; then
  echo "=== netem matrix (linux root) ===" | tee -a "${LOG}"
  mkdir -p docs/RESULTS
  cargo run --release -p sct-bench -- netem-matrix \
    --interface lo --profile all --sizes-mib 1,16,256,1024 \
    --output-json "docs/RESULTS/$(date +%F)-linux-netem-matrix.json" 2>&1 | tee -a "${LOG}"
else
  echo "=== netem matrix skipped (requires Linux+root) ===" | tee -a "${LOG}"
fi

echo "finished=$(date -u +%Y-%m-%dT%H:%M:%SZ)" | tee -a "${LOG}"
echo "Done. Artifacts: ${SC_TRANSPORT_ARTIFACT_DIR}/"
