#!/bin/bash
set -euo pipefail
exec > /var/log/sct-cloud-init.log 2>&1

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq && apt-get install -y curl build-essential pkg-config libssl-dev git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
# shellcheck disable=SC1091
source /root/.cargo/env
git clone --depth 1 https://github.com/SynapticFour/sc-transport /opt/sct
cd /opt/sct && cargo build --release -p sct-cli
cp target/release/sct /usr/local/bin/sct
install -m 0755 scripts/wan_test.sh /usr/local/bin/wan_test.sh
echo "SENDER_READY" > /tmp/sct-ready
