#!/bin/bash
# Run WAN transfer test on AWS EC2 sender → receiver (Frankfurt → Ashburn).
# Requires: instances started via start_wan_test.sh, SSH key matching sct-wan-key.
set -euo pipefail

SENDER_ID=$(grep eu-central-1 /tmp/sct-wan-instances.txt | cut -d' ' -f1)
RECEIVER_ID=$(grep us-east-1 /tmp/sct-wan-instances.txt | cut -d' ' -f1)

SENDER_IP=$(aws ec2 describe-instances --region eu-central-1 \
    --instance-ids "$SENDER_ID" \
    --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)
RECEIVER_IP=$(aws ec2 describe-instances --region us-east-1 \
    --instance-ids "$RECEIVER_ID" \
    --query 'Reservations[0].Instances[0].PublicIpAddress' --output text)

SSH_KEY="${SCT_WAN_SSH_PRIVATE_KEY_PATH:-$HOME/.ssh/sct-wan-key.pem}"
if [[ -n "${SCT_WAN_SSH_PRIVATE_KEY:-}" ]]; then
  printf '%s\n' "$SCT_WAN_SSH_PRIVATE_KEY" > /tmp/sct-wan-ssh-key
  chmod 600 /tmp/sct-wan-ssh-key
  SSH_KEY=/tmp/sct-wan-ssh-key
fi

SSH_OPTS=(-o StrictHostKeyChecking=no -o ConnectTimeout=15 -i "$SSH_KEY")

wait_ready() {
  local host="$1" label="$2"
  for i in $(seq 1 60); do
    if ssh "${SSH_OPTS[@]}" "ubuntu@$host" test -f /tmp/sct-ready 2>/dev/null; then
      echo "$label ready after $((i * 10))s"
      return 0
    fi
    sleep 10
  done
  echo "ERROR: $label not ready in time" >&2
  return 1
}

echo "Sender:   $SENDER_IP (eu-central-1)"
echo "Receiver: $RECEIVER_IP (us-east-1)"
wait_ready "$SENDER_IP" "Sender"
wait_ready "$RECEIVER_IP" "Receiver"

mkdir -p wan-results
ssh "${SSH_OPTS[@]}" "ubuntu@$SENDER_IP" \
  "SENDER_REGION=eu-central-1 RECEIVER_REGION=us-east-1 \
   wan_test.sh '$RECEIVER_IP' /tmp/wan-results 9410"

scp "${SSH_OPTS[@]}" "ubuntu@$SENDER_IP:/tmp/wan-results/*" ./wan-results/ || true
cat wan-results/results.txt 2>/dev/null || true
