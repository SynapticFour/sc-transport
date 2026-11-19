#!/bin/bash
# Runs ON the sender VM as root. Closes the open Linux items in
# docs/PRODUCTION-READINESS-CHECKLIST.md section A: kernel tc netem, loss tests,
# fallback above 15 % loss, benches with and without netem, three independent runs.
# Usage: run_campaign.sh <output dir>
set -euo pipefail

OUT="${1:?output dir}"
RUNS="${SCT_CAMPAIGN_RUNS:-3}"
FEATURES=transport-quic,transport-datagrams
cd /opt/sct
# shellcheck disable=SC1091
source /root/.cargo/env
mkdir -p "$OUT"
exec > >(tee -a "$OUT/campaign.log") 2>&1

shape_off() { tc qdisc del dev lo root >/dev/null 2>&1 || true; }
shape() { shape_off; tc qdisc add dev lo root netem "$@"; }
trap shape_off EXIT

# Refuse to report numbers when the kernel cannot shape.
shape delay 1ms || { echo "STOP: tc netem unavailable on $(uname -r)"; exit 1; }
shape_off
echo "kernel=$(uname -r) commit=$(git rev-parse HEAD) runs=$RUNS" | tee "$OUT/environment.txt"

# One release-build transfer over loopback; prints elapsed ms or FAIL.
transfer() { # name size_mib
  local dir="/tmp/sct-campaign-$1" port=7272
  rm -rf "$dir"; mkdir -p "$dir/recv"
  head -c "$(( $2 * 1048576 ))" /dev/urandom > "$dir/payload.bin"
  sct recv --port "$port" --output-dir "$dir/recv" --once > "$dir/recv.log" 2>&1 &
  local rpid=$!; sleep 1
  local start; start=$(date +%s%N)
  if sct send "$dir/payload.bin" "sct://127.0.0.1:$port" --compression none --quiet > "$dir/send.log" 2>&1 \
     && wait "$rpid" \
     && cmp -s "$dir/payload.bin" "$dir/recv/$(ls -1 "$dir/recv" | head -n 1)"; then
    echo $(( ($(date +%s%N) - start) / 1000000 ))
  else
    kill "$rpid" 2>/dev/null || true; echo FAIL
  fi
}

cargo build --release -p sc-transport --features "$FEATURES" --tests >/dev/null

for run in $(seq 1 "$RUNS"); do
  echo "== run $run/$RUNS: netem matrix (good, bad, very-bad)"
  cargo run --release -p sct-bench -- netem-matrix --interface lo --profile all \
    --sizes-mib 16,256,1024 --output-json "$OUT/run-$run-netem-matrix.json"

  echo "== run $run/$RUNS: loss and fallback tests under kernel netem"
  for loss in 5 15 20; do
    shape loss "${loss}%"
    if cargo test --release -p sc-transport --features "$FEATURES" \
         --test datagram_fallback_trigger > "$OUT/run-$run-loss-$loss.log" 2>&1; then
      echo "loss=$loss% PASS"
    else
      echo "loss=$loss% FAIL (see run-$run-loss-$loss.log)"
    fi
    shape_off
  done

  echo "== run $run/$RUNS: latency and throughput benches without and with netem"
  for mode in plain shaped; do
    [ "$mode" = shaped ] && shape delay 50ms 5ms loss 1%
    cargo bench -p sc-transport --features "$FEATURES" --bench latency --bench throughput \
      > "$OUT/run-$run-bench-$mode.log" 2>&1 && echo "bench $mode done" || echo "bench $mode FAIL"
    shape_off
  done

  echo "== run $run/$RUNS: 1 GiB / 256 MiB transfers without and with netem"
  echo "baseline run=$run elapsed_ms=$(transfer baseline 1024)" | tee -a "$OUT/regional.txt"
  while IFS='|' read -r name delay jitter loss rate; do
    case "$name" in ''|'#'*) continue ;; esac
    shape delay "${delay}ms" "${jitter}ms" loss "${loss}%" rate "${rate}mbit"
    echo "$name run=$run elapsed_ms=$(transfer "$name" 256)" | tee -a "$OUT/regional.txt"
    shape_off
  done < /opt/sct/infra/azure/profiles.txt
done
echo "Campaign finished: $OUT"
