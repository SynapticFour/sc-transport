#!/usr/bin/env bash
set -euo pipefail

if [[ "${OSTYPE:-}" != linux* ]]; then
  echo "netem runner supports Linux only; skipping."
  exit 0
fi

INTERFACE="${1:-lo}"
LOSS_PERCENT="${2:-20}"
TEST_NAME="${3:-datagram_fallback_trigger}"

cleanup() {
  sudo tc qdisc del dev "${INTERFACE}" root >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "Applying netem loss=${LOSS_PERCENT}% on interface=${INTERFACE}"
sudo tc qdisc add dev "${INTERFACE}" root netem loss "${LOSS_PERCENT}%"

echo "Running test: ${TEST_NAME}"
cargo test --test "${TEST_NAME}"
