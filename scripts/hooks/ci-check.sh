#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
echo "ci-check: cargo fmt --check"
cargo fmt --all -- --check
echo "ci-check: cargo clippy"
cargo clippy --workspace --all-targets --all-features -- -D warnings
echo "ci-check: cargo test"
# cli_smoke / daemon_smoke spawn sct + sct-daemon; cargo test does not
# emit those bins (GitHub CI builds them in scripts/ci-cli-daemon-smoke.sh).
cargo build -p sct-cli -p sct-daemon
target_dir="$(cargo metadata --format-version=1 --no-deps 2>/dev/null \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | head -1)"
target_dir="${target_dir:-${CARGO_TARGET_DIR:-target}}"
export SCT_SMOKE_BIN="${target_dir}/debug/sct"
export SCT_SMOKE_DAEMON_BIN="${target_dir}/debug/sct-daemon"
cargo test --workspace
echo "ci-check: OK"
