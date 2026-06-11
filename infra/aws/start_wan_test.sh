#!/bin/bash
set -euo pipefail

# Ubuntu 22.04 ARM AMI (arm64, eu-central-1)
AMI_EU="ami-0c4b6e35e8e80cde3"
# Ubuntu 22.04 ARM AMI (arm64, us-east-1)
AMI_US="ami-0b77c4a8a42a2b1e3"

# Sender: Frankfurt (eu-central-1)
echo "Starting sender in Frankfurt..."
SENDER_ID=$(aws ec2 run-instances \
    --region eu-central-1 \
    --image-id $AMI_EU \
    --instance-type t4g.small \
    --key-name sct-wan-key \
    --security-group-ids sg-sct-wan \
    --user-data file://infra/aws/cloud-init-sender.sh \
    --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=sct-wan-sender}]' \
    --query 'Instances[0].InstanceId' \
    --output text)

# Receiver: Ashburn (us-east-1)
echo "Starting receiver in Ashburn..."
RECEIVER_ID=$(aws ec2 run-instances \
    --region us-east-1 \
    --image-id $AMI_US \
    --instance-type t4g.small \
    --key-name sct-wan-key \
    --security-group-ids sg-sct-wan \
    --user-data file://infra/aws/cloud-init-receiver.sh \
    --tag-specifications 'ResourceType=instance,Tags=[{Key=Name,Value=sct-wan-receiver}]' \
    --query 'Instances[0].InstanceId' \
    --output text)

echo "Sender: $SENDER_ID (eu-central-1)"
echo "Receiver: $RECEIVER_ID (us-east-1)"
echo "IDs gespeichert in /tmp/sct-wan-instances.txt"
echo "$SENDER_ID eu-central-1" > /tmp/sct-wan-instances.txt
echo "$RECEIVER_ID us-east-1" >> /tmp/sct-wan-instances.txt
echo ""
echo "Warte ~3 Minuten, dann:"
echo "  bash infra/aws/run_test.sh"
