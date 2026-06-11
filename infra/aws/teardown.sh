#!/bin/bash
set -euo pipefail
# Beide Instanzen terminieren — IMMER ausführen nach dem Test!
SENDER_ID=$(grep "eu-central-1" /tmp/sct-wan-instances.txt | cut -d' ' -f1)
RECEIVER_ID=$(grep "us-east-1" /tmp/sct-wan-instances.txt | cut -d' ' -f1)
aws ec2 terminate-instances --region eu-central-1 --instance-ids $SENDER_ID
aws ec2 terminate-instances --region us-east-1  --instance-ids $RECEIVER_ID
echo "Instanzen terminiert. Kosten: ~\$0.034"
