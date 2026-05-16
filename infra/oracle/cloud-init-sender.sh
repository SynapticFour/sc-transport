#!/bin/bash
# Terraform template variables: github_repo, github_ref, sct_port
set -euo pipefail
exec > /var/log/sct-cloud-init.log 2>&1

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y curl build-essential pkg-config libssl-dev git python3

# Rust
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
# shellcheck disable=SC1091
source /root/.cargo/env
export PATH="/root/.cargo/bin:$PATH"

rm -rf /opt/sct
git clone --depth 1 --branch "${github_ref}" \
  "https://github.com/${github_repo}.git" /opt/sct
cd /opt/sct
cargo build --release -p sct-cli
install -m 0755 target/release/sct /usr/local/bin/sct

# Copy WAN test script for GitHub Actions SSH step
install -m 0755 scripts/wan_test.sh /usr/local/bin/wan_test.sh

echo "SCT_SENDER_READY" > /tmp/sct-ready
