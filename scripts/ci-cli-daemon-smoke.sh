#!/usr/bin/env bash
# sct-cli / sct-daemon subprocess smoke tests (loopback QUIC + REST).
set -euo pipefail

cd "$(dirname "$0")/.."

export RUST_TEST_THREADS=1

echo "==> build sct + sct-daemon"
cargo build -p sct-cli -p sct-daemon --quiet

target_dir="$(cargo metadata --format-version=1 --no-deps 2>/dev/null \
  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | head -1)"
target_dir="${target_dir:-${CARGO_TARGET_DIR:-target}}"
profile="${PROFILE:-debug}"
export SCT_SMOKE_BIN="${target_dir}/${profile}/sct"
export SCT_SMOKE_DAEMON_BIN="${target_dir}/${profile}/sct-daemon"

for bin in "$SCT_SMOKE_BIN" "$SCT_SMOKE_DAEMON_BIN"; do
  if [[ ! -x "$bin" ]]; then
    echo "missing executable: $bin" >&2
    exit 1
  fi
done

echo "==> sc-transport --test cli_smoke"
cargo test -p sc-transport --test cli_smoke -- --test-threads=1

echo "==> sc-transport --test daemon_smoke"
cargo test -p sc-transport --test daemon_smoke -- --test-threads=1

echo "cli/daemon smoke tests passed"
