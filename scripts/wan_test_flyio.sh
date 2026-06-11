#!/bin/bash
set -euo pipefail
# Fly.io WAN Test: Frankfurt (fra) → Ashburn (iad)
# Voraussetzung: fly auth login, apps erstellt (siehe infra/flyio/README.md)

RESULTS_DIR="results/wan-flyio-$(date +%Y-%m-%d)"
mkdir -p "$RESULTS_DIR"

echo "=== SCT WAN Test: Frankfurt → Ashburn (Fly.io) ==="

# Receiver starten (Ashburn)
echo "Starting receiver in iad..."
fly machine run --app sct-wan-receiver \
    --region iad \
    --config infra/flyio/fly-receiver.toml \
    --detach

RECEIVER_IP=$(fly ips list --app sct-wan-receiver --json | \
    python3 -c "import json,sys; print(json.load(sys.stdin)[0]['Address'])")
echo "Receiver IP: $RECEIVER_IP"

# Kurz warten
sleep 10

# Sender: Test von Frankfurt aus ausführen
for SIZE_MB in 1 10 100; do
    echo "Testing ${SIZE_MB}MB transfer..."
    fly machine run --app sct-wan-sender \
        --region fra \
        --config infra/flyio/fly-sender.toml \
        --command "bash -c '
            dd if=/dev/urandom of=/tmp/test.bin bs=1M count=${SIZE_MB} 2>/dev/null
            START=\$(date +%s%N)
            sct send /tmp/test.bin ${RECEIVER_IP}:9410 --compression zstd
            END=\$(date +%s%N)
            ELAPSED=\$(( (END-START)/1000000 ))
            echo \"${SIZE_MB}mb: \${ELAPSED}ms\"
        '" | tee -a "$RESULTS_DIR/results.txt"
done

# Aufräumen
fly machine stop --app sct-wan-sender --all
fly machine stop --app sct-wan-receiver --all

echo "Ergebnisse: $RESULTS_DIR/results.txt"
