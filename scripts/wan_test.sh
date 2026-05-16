#!/bin/bash
# WAN transfer benchmark: run on sender VM against a remote sct recv endpoint.
# Usage: wan_test.sh <receiver_ip> [results_dir] [sct_port]
set -euo pipefail

RECEIVER_IP="${1:?receiver IP required}"
RESULTS_DIR="${2:-/tmp/wan-results}"
SCT_PORT="${3:-9410}"
mkdir -p "$RESULTS_DIR"

SENDER_REGION="${SENDER_REGION:-unknown}"
RECEIVER_REGION="${RECEIVER_REGION:-unknown}"

echo "=== SCT WAN Test: ${SENDER_REGION} → ${RECEIVER_REGION} ==="
echo "Receiver: ${RECEIVER_IP}:${SCT_PORT}"
: > "$RESULTS_DIR/metadata.txt"
: > "$RESULTS_DIR/results.txt"

echo "sender_region=${SENDER_REGION}" >> "$RESULTS_DIR/metadata.txt"
echo "receiver_region=${RECEIVER_REGION}" >> "$RESULTS_DIR/metadata.txt"
echo "receiver_ip=${RECEIVER_IP}" >> "$RESULTS_DIR/metadata.txt"
echo "sct_port=${SCT_PORT}" >> "$RESULTS_DIR/metadata.txt"

if ping -c 10 -W 2 "$RECEIVER_IP" >/tmp/ping.out 2>&1; then
  RTT=$(awk -F'/' '/^rtt |^round-trip/ {print $5}' /tmp/ping.out | head -1)
  echo "avg_rtt_ms=${RTT:-na}" >> "$RESULTS_DIR/metadata.txt"
else
  echo "avg_rtt_ms=unreachable" >> "$RESULTS_DIR/metadata.txt"
fi

ENDPOINT="${RECEIVER_IP}:${SCT_PORT}"

for SIZE_MB in 1 10 100; do
  FILE="/tmp/test_${SIZE_MB}mb.bin"
  dd if=/dev/urandom of="$FILE" bs=1M count="$SIZE_MB" status=none

  START=$(date +%s%N)
  sct send "$FILE" "$ENDPOINT" --compression zstd --parallel 16
  END=$(date +%s%N)

  ELAPSED_MS=$(( (END - START) / 1000000 ))
  THROUGHPUT_MBPS=$(python3 -c "print(f'{$SIZE_MB * 8000 / max($ELAPSED_MS, 1):.2f}')")

  echo "${SIZE_MB}mb: elapsed=${ELAPSED_MS}ms throughput=${THROUGHPUT_MBPS}Mbps"
  echo "wan_${SIZE_MB}mb_elapsed_ms=$ELAPSED_MS" >> "$RESULTS_DIR/results.txt"
  echo "wan_${SIZE_MB}mb_throughput_mbps=$THROUGHPUT_MBPS" >> "$RESULTS_DIR/results.txt"
  rm -f "$FILE"
done

cat "$RESULTS_DIR/results.txt"
