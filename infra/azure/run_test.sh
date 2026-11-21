#!/bin/bash
# Run the WAN transfer test (real path) and, with --campaign, the netem campaign
# on the sender VM. Results land in ./wan-results/<UTC timestamp>/.
set -euo pipefail

STATE="${SCT_AZ_STATE:-/tmp/sct-azure-wan.env}"
# shellcheck disable=SC1090
source "$STATE"
CAMPAIGN=false
[ "${1:-}" = "--campaign" ] && CAMPAIGN=true

SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/tmp/sct-azure-known-hosts
          -o ConnectTimeout=15 -i "$SSH_KEY")
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
wait_ready "$RECEIVER_IP" Receiver

OUT="wan-results/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$OUT"
{
  echo "date_utc=$(date -u +%FT%TZ)"
  echo "vm_size=$SIZE"; echo "arch=$ARCH"
  echo "sender_commit=$(on "$SENDER_IP" cat /opt/sct-commit)"
  echo "receiver_commit=$(on "$RECEIVER_IP" cat /opt/sct-commit)"
  echo "sender_kernel=$(on "$SENDER_IP" uname -r)"
  # Azure drops ICMP to public IPs, so measure RTT as TCP connect time to sshd.
  echo "tcp_connect_rtt_ms=$(on "$SENDER_IP" "python3 -c \"import socket,time
s=[]
for _ in range(10):
    t=time.perf_counter(); c=socket.create_connection(('$RECEIVER_IP',22),5); s.append((time.perf_counter()-t)*1000); c.close()
s.sort(); print(round(s[len(s)//2],1))\"" 2>/dev/null || echo na)"
} > "$OUT/environment.txt"

echo "== WAN: $SENDER_LOCATION -> $RECEIVER_LOCATION"
for run in 1 2 3; do
  on "$SENDER_IP" "SENDER_REGION=$SENDER_LOCATION RECEIVER_REGION=$RECEIVER_LOCATION \
    wan_test.sh '$RECEIVER_IP' /tmp/wan-results-$run $PORT"
  mkdir -p "$OUT/wan-run-$run"
  scp "${SSH_OPTS[@]}" "$ADMIN@$SENDER_IP:/tmp/wan-results-$run/*" "$OUT/wan-run-$run/"
done

if $CAMPAIGN; then
  echo "== netem campaign on the sender (loopback, kernel netem)"
  on "$SENDER_IP" "sudo -i run_campaign.sh /tmp/campaign"
  on "$SENDER_IP" "sudo tar -C /tmp -czf /tmp/campaign.tgz campaign"
  scp "${SSH_OPTS[@]}" "$ADMIN@$SENDER_IP:/tmp/campaign.tgz" "$OUT/"
fi
cat "$OUT/environment.txt"
echo "Results in $OUT. Next: bash infra/azure/teardown.sh"
