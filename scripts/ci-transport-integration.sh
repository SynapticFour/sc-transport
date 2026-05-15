#!/usr/bin/env bash
# sc-transport integration tests for CI / make ci-transport-integration.
set -euo pipefail

cd "$(dirname "$0")/.."

export RUST_TEST_THREADS=1
export SC_TRANSPORT_ALLOW_INSECURE_QUIC=true
export SC_QUIC_CONNECT_TIMEOUT_MS="${SC_QUIC_CONNECT_TIMEOUT_MS:-2000}"
export SC_QUIC_MIRROR_SSE="${SC_QUIC_MIRROR_SSE:-1}"

run_test() {
  local features="$1"
  local name="$2"
  echo "==> sc-transport --test ${name} (features=${features})"
  cargo test -p sc-transport --features "${features}" --test "${name}" -- --test-threads=1
}

run_test transport-sse sse_basic
run_test transport-quic sse_fallback
run_test transport-quic quic_stream_basic
run_test transport-quic quic_stream_reconnect
run_test transport-datagrams datagram_delivery_no_loss
run_test transport-datagrams datagram_delivery_5pct_loss
run_test transport-datagrams datagram_delivery_20pct_loss
run_test transport-datagrams datagram_fallback_trigger
run_test transport-quic,transport-datagrams transport_transparency
run_test transport-quic,transport-datagrams wan_simulation

# Matrix smoke: excellent × tiny/small (full cross-product is local/bench only).
export SC_STREAM_MATRIX_QUALITIES="${SC_STREAM_MATRIX_QUALITIES:-excellent}"
export SC_STREAM_MATRIX_PAYLOADS="${SC_STREAM_MATRIX_PAYLOADS:-tiny,small}"
export SC_STREAM_MATRIX_REPEATS="${SC_STREAM_MATRIX_REPEATS:-2}"
run_test transport-quic,transport-datagrams streaming_matrix

echo "sc-transport integration tests passed"
