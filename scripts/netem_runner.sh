#!/usr/bin/env bash
set -euo pipefail

if [[ "${OSTYPE:-}" != linux* ]]; then
  echo "netem runner supports Linux only; skipping."
  exit 0
fi

COMMAND="${1:-single}"
INTERFACE="${2:-lo}"
LOSS_PERCENT="${3:-20}"
TEST_NAME="${4:-datagram_fallback_trigger}"

cleanup() {
  if [[ "$(id -u)" -eq 0 ]]; then
    tc qdisc del dev "${INTERFACE}" root >/dev/null 2>&1 || true
  else
    sudo tc qdisc del dev "${INTERFACE}" root >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

single_run() {
  echo "Applying netem loss=${LOSS_PERCENT}% on interface=${INTERFACE}"
  if [[ "$(id -u)" -eq 0 ]]; then
    tc qdisc add dev "${INTERFACE}" root netem loss "${LOSS_PERCENT}%"
  else
    sudo tc qdisc add dev "${INTERFACE}" root netem loss "${LOSS_PERCENT}%"
  fi
  echo "Running test: ${TEST_NAME}"
  cargo test --test "${TEST_NAME}"
}

setup_netem_continental() {
  LOSS_PERCENT="0.1"
  if [[ "$(id -u)" -eq 0 ]]; then
    tc qdisc add dev "${INTERFACE}" root netem delay 50ms 5ms loss "${LOSS_PERCENT}%"
  else
    sudo tc qdisc add dev "${INTERFACE}" root netem delay 50ms 5ms loss "${LOSS_PERCENT}%"
  fi
}

setup_netem_intercontinental() {
  LOSS_PERCENT="0.5"
  if [[ "$(id -u)" -eq 0 ]]; then
    tc qdisc add dev "${INTERFACE}" root netem delay 100ms 10ms loss "${LOSS_PERCENT}%"
  else
    sudo tc qdisc add dev "${INTERFACE}" root netem delay 100ms 10ms loss "${LOSS_PERCENT}%"
  fi
}

setup_netem_degraded() {
  LOSS_PERCENT="2.0"
  if [[ "$(id -u)" -eq 0 ]]; then
    tc qdisc add dev "${INTERFACE}" root netem delay 150ms 20ms loss "${LOSS_PERCENT}%"
  else
    sudo tc qdisc add dev "${INTERFACE}" root netem delay 150ms 20ms loss "${LOSS_PERCENT}%"
  fi
}

teardown_netem() {
  cleanup
}

run_wan_scenarios() {
  for scenario in continental intercontinental degraded; do
    echo "=== WAN scenario: $scenario ==="
    teardown_netem || true
    setup_netem_"$scenario"
    cargo test wan_simulation --features transport-quic,transport-datagrams -- --nocapture
    teardown_netem
  done
}

if [[ "${COMMAND}" == "run_wan_scenarios" ]]; then
  run_wan_scenarios
else
  single_run
fi
