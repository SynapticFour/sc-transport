#!/bin/bash
# Prepares an Ubuntu 24.04 host (any provider) as WAN test sender or receiver.
# Runs as root, either as cloud-init user data (the start scripts prepend
# `SCT_ROLE=sender|receiver`, `SCT_REF=<git ref>` and `SCT_PORT=<port>`) or over SSH:
#   ssh root@HOST 'SCT_ROLE=sender SCT_REF=main bash -s' < infra/linux/prepare_host.sh
# Writes /tmp/sct-ready when done. Log: /var/log/sct-cloud-init.log
set -euo pipefail
exec > /var/log/sct-cloud-init.log 2>&1

SCT_ROLE="${SCT_ROLE:?SCT_ROLE missing}"
SCT_REF="${SCT_REF:-main}"
SCT_PORT="${SCT_PORT:-9410}"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y curl build-essential pkg-config libssl-dev git iproute2 python3 jq
# Some cloud kernels (Azure, GCP) ship sch_netem in linux-modules-extra; without it
# `tc ... netem` fails. Others (Hetzner, generic) have it built in.
apt-get install -y "linux-modules-extra-$(uname -r)" || true
modprobe sch_netem || echo "WARN: sch_netem not loadable; netem campaign will refuse to run"

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
# shellcheck disable=SC1091
source /root/.cargo/env
git clone https://github.com/SynapticFour/sc-transport /opt/sct
cd /opt/sct
git checkout --detach "${SCT_REF}"
cargo build --release -p sct-cli
install -m 0755 target/release/sct /usr/local/bin/sct
git rev-parse HEAD > /opt/sct-commit

if [ "${SCT_ROLE}" = "receiver" ]; then
  mkdir -p /data
  nohup sct recv --port "${SCT_PORT}" --output-dir /data --daemon >> /var/log/sct-recv.log 2>&1 &
else
  install -m 0755 scripts/wan_test.sh /usr/local/bin/wan_test.sh
  install -m 0755 infra/linux/run_campaign.sh /usr/local/bin/run_campaign.sh
  # Pre-build the bench so the campaign measures transfers, not compilation.
  cargo build --release -p sct-bench
fi
echo "${SCT_ROLE^^}_READY" > /tmp/sct-ready
