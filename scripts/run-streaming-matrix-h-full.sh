#!/usr/bin/env bash
# Full streaming matrix (all qualities × payloads × transports) → results/H-full,
# then competitive baseline + compare vs results/streaming-matrix-summary-G.json.
#
# Usage (from repo root = sc-transport/):
#   bash scripts/run-streaming-matrix-h-full.sh
#
# Optional env:
#   STEP_RUN_CI=0          — skip scripts/ci-transport-integration.sh (default: 1 = run)
#   SC_TRANSPORT_ARTIFACT_DIR — default: results/H-full
#   SC_STREAM_MATRIX_REPEATS  — default: 5 (match typical G baseline repeats)
#   CARGO_PROFILE           — default: --release (set to empty for debug)
#
set -euo pipefail

cd "$(dirname "$0")/.."

: "${SC_TRANSPORT_ARTIFACT_DIR:=results/H-full}"
: "${SC_STREAM_MATRIX_REPEATS:=5}"
: "${STEP_RUN_CI:=1}"
CARGO_PROFILE="${CARGO_PROFILE:---release}"

export SC_TRANSPORT_ALLOW_INSECURE_QUIC="${SC_TRANSPORT_ALLOW_INSECURE_QUIC:-true}"
export SC_QUIC_MIRROR_SSE="${SC_QUIC_MIRROR_SSE:-1}"
export RUST_TEST_THREADS=1
export SC_TRANSPORT_ARTIFACT_DIR
export SC_STREAM_MATRIX_REPEATS

mkdir -p "${SC_TRANSPORT_ARTIFACT_DIR}"
RAW="${SC_TRANSPORT_ARTIFACT_DIR}/streaming-matrix-raw.txt"

if [[ "${STEP_RUN_CI}" == "1" ]]; then
  bash scripts/ci-transport-integration.sh
fi

# Full matrix: omit SC_STREAM_MATRIX_QUALITIES / SC_STREAM_MATRIX_PAYLOADS → all 4×4.
# shellcheck disable=SC2086
cargo test ${CARGO_PROFILE} -p sc-transport --features transport-quic,transport-datagrams \
  --test streaming_matrix -- --nocapture 2>&1 | tee "${RAW}"

python3 scripts/competitive_baseline_matrix.py \
  --sc-summary "${SC_TRANSPORT_ARTIFACT_DIR}/streaming-matrix-summary.json" \
  --out-json "${SC_TRANSPORT_ARTIFACT_DIR}/competitive-baseline-matrix.json" \
  --out-md "${SC_TRANSPORT_ARTIFACT_DIR}/competitive-baseline-matrix.md"

python3 scripts/compare_streaming_matrix.py \
  --before results/streaming-matrix-summary-G.json \
  --after "${SC_TRANSPORT_ARTIFACT_DIR}/streaming-matrix-summary.json" \
  --out-md results/streaming-matrix-compare-G-vs-H-full.md \
  --out-json results/streaming-matrix-compare-G-vs-H-full.json

echo "Done. Artifacts under ${SC_TRANSPORT_ARTIFACT_DIR}/ and results/streaming-matrix-compare-G-vs-H-full.{md,json}"
