#!/bin/bash
# Versucht mehrere Regionen bis Kapazität verfügbar ist.
# Typischer Fehler: "Out of host capacity" — wechsle Region oder AD.
set -euo pipefail

cd "$(dirname "$0")"

SENDER_REGIONS=(
  "eu-frankfurt-1"
  "eu-amsterdam-1"
  "us-ashburn-1"
  "ca-toronto-1"
  "ap-tokyo-1"
)

RECEIVER_REGIONS=(
  "us-ashburn-1"
  "eu-frankfurt-1"
  "eu-amsterdam-1"
  "ca-toronto-1"
  "ap-tokyo-1"
)

for i in "${!SENDER_REGIONS[@]}"; do
  REGION="${SENDER_REGIONS[$i]}"
  RECEIVER="${RECEIVER_REGIONS[$i]:-us-ashburn-1}"
  echo "Trying sender region: $REGION (receiver: $RECEIVER)"
  export TF_VAR_sender_region="$REGION"
  export TF_VAR_receiver_region="$RECEIVER"

  if terraform apply -auto-approve; then
    echo "Provisioned in sender=$REGION receiver=$RECEIVER"
    exit 0
  fi

  echo "Failed in sender=$REGION receiver=$RECEIVER, cleaning up..."
  terraform destroy -auto-approve || true
  sleep 10
done

echo "All regions exhausted. Try manually at cloud.oracle.com"
exit 1
