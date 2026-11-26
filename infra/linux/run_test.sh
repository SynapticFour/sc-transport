#!/bin/bash
# Runs the WAN transfer test over the real path and, with --campaign, the netem
# campaign on the sender. --campaign-only runs the campaign alone; it needs only a
# sender. Works for every provider: the start scripts (infra/azure, infra/hetzner,
# infra/linux/hosts.sh) write the state file this script reads.
# Results land in ./wan-results/<UTC timestamp>/.
set -euo pipefail

STATE="${SCT_STATE:-${SCT_AZ_STATE:-/tmp/sct-wan.env}}"
# shellcheck disable=SC1090
source "$STATE"
CAMPAIGN=false
WAN=true
case "${1:-}" in
  --campaign) CAMPAIGN=true ;;
  --campaign-only) CAMPAIGN=true; WAN=false ;;
  "") ;;
  *) echo "usage: run_test.sh [--campaign|--campaign-only]" >&2; exit 2 ;;
esac
SIZE="${SIZE:-unknown}"
ARCH="${ARCH:-unknown}"
PORT="${PORT:-9410}"

SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/tmp/sct-wan-known-hosts
          -o ConnectTimeout=15)
[ -n "${SSH_KEY:-}" ] && SSH_OPTS+=(-i "$SSH_KEY")
on() { local host="$1"; shift; ssh "${SSH_OPTS[@]}" "$ADMIN@$host" "$@"; }

wait_ready() {
  for i in $(seq 1 90); do
    if on "$1" test -f /tmp/sct-ready 2>/dev/null; then echo "$2 ready after $((i * 10))s"; return 0; fi
    sleep 10
  done
  echo "ERROR: $2 not ready in 15 minutes; see /var/log/sct-cloud-init.log on the VM" >&2
  return 1
}
wait_ready "$SENDER_IP" Sender
$WAN && wait_ready "$RECEIVER_IP" Receiver

OUT="wan-results/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$OUT"
{
  echo "date_utc=$(date -u +%FT%TZ)"
  echo "vm_size=$SIZE"; echo "arch=$ARCH"
  echo "provider=${PROVIDER:-unknown}"
  echo "sender_location=${SENDER_LOCATION:-unknown}"
  echo "sender_commit=$(on "$SENDER_IP" cat /opt/sct-commit)"
  echo "sender_kernel=$(on "$SENDER_IP" uname -r)"
  if $WAN; then
  echo "receiver_location=${RECEIVER_LOCATION:-unknown}"
  echo "receiver_commit=$(on "$RECEIVER_IP" cat /opt/sct-commit)"
  # Some clouds drop ICMP to public IPs, so measure RTT as TCP connect time to sshd.
  echo "tcp_connect_rtt_ms=$(on "$SENDER_IP" "python3 -c \"import socket,time
s=[]
for _ in range(10):
    t=time.perf_counter(); c=socket.create_connection(('$RECEIVER_IP',22),5); s.append((time.perf_counter()-t)*1000); c.close()
s.sort(); print(round(s[len(s)//2],1))\"" 2>/dev/null || echo na)"
  fi
} > "$OUT/environment.txt"

if $WAN; then
echo "== WAN: ${SENDER_LOCATION:-?} -> ${RECEIVER_LOCATION:-?}"
for run in 1 2 3; do
  on "$SENDER_IP" "SENDER_REGION=${SENDER_LOCATION:-unknown} RECEIVER_REGION=${RECEIVER_LOCATION:-unknown} \
    wan_test.sh '$RECEIVER_IP' /tmp/wan-results-$run $PORT"
  mkdir -p "$OUT/wan-run-$run"
  scp "${SSH_OPTS[@]}" "$ADMIN@$SENDER_IP:/tmp/wan-results-$run/*" "$OUT/wan-run-$run/"
done
fi

if $CAMPAIGN; then
  echo "== netem campaign on the sender (loopback, kernel netem)"
  on "$SENDER_IP" "sudo -i run_campaign.sh /tmp/campaign"
  on "$SENDER_IP" "sudo tar -C /tmp -czf /tmp/campaign.tgz campaign"
  scp "${SSH_OPTS[@]}" "$ADMIN@$SENDER_IP:/tmp/campaign.tgz" "$OUT/"
fi
cat "$OUT/environment.txt"
echo "Results in $OUT. Next: the teardown of the provider you used (${TEARDOWN:-see infra/linux/README.md})."
