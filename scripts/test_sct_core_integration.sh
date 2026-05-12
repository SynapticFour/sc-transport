#!/usr/bin/env bash
# Run sct-core integration test binaries one after another (QUIC loopback + tokio
# tests contend if many binaries run in parallel under `cargo test -p sct-core`).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export RUST_TEST_THREADS=1
for t in completion_campaign completion_first_ab prompt5_integration transfer_smoke; do
  echo "==> cargo test -p sct-core --test $t"
  cargo test -p sct-core --test "$t" -- --test-threads=1
done
echo "==> all sct-core integration tests passed"
