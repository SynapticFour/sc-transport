#!/bin/bash
# Bring your own hosts: prepares one or two Ubuntu 24.04 hosts you already have
# (any provider: Google, Hostman, Vultr, Contabo, a lab server) and writes the state
# file that infra/linux/run_test.sh reads. Nothing is created or deleted.
#
# Usage:
#   bash infra/linux/hosts.sh SENDER [RECEIVER]
#   SENDER, RECEIVER: user@address with SSH access as root or with passwordless sudo.
#   Without RECEIVER only the netem campaign can run (run_test.sh --campaign-only).
#
# Env (all optional):
#   SCT_REF            git ref the hosts build (default: current commit if pushed, else main)
#   SCT_STATE          state file (default /tmp/sct-wan.env)
#   SCT_SSH_KEY        private key for SSH (default: your SSH agent / config)
#   SENDER_LOCATION, RECEIVER_LOCATION, PROVIDER   labels for environment.txt
#
# Firewall: the receiver must accept UDP 9410 from the sender. Open it in the
# provider's firewall before the WAN test; this script does not change firewalls.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SENDER="${1:?usage: hosts.sh user@sender [user@receiver]}"
RECEIVER="${2:-}"
STATE="${SCT_STATE:-/tmp/sct-wan.env}"
PORT=9410

if [ -z "${SCT_REF:-}" ]; then
  head="$(git -C "$HERE" rev-parse HEAD 2>/dev/null || true)"
  if [ -n "$head" ] && git -C "$HERE" branch -r --contains "$head" 2>/dev/null | grep -q .; then
    SCT_REF="$head"
  else
    SCT_REF=main
  fi
fi

SSH_OPTS=(-o StrictHostKeyChecking=accept-new -o UserKnownHostsFile=/tmp/sct-wan-known-hosts
          -o ConnectTimeout=15)
[ -n "${SCT_SSH_KEY:-}" ] && SSH_OPTS+=(-i "$SCT_SSH_KEY")

user_of() { echo "${1%@*}"; }
addr_of() { echo "${1#*@}"; }
[ "$(user_of "$SENDER")" != "$SENDER" ] || { echo "SENDER must be user@address" >&2; exit 2; }
if [ -n "$RECEIVER" ] && [ "$(user_of "$RECEIVER")" != "$(user_of "$SENDER")" ]; then
  echo "Sender and receiver must use the same SSH user" >&2; exit 2
fi

prepare() { # user@host role
  echo "Preparing $1 as $2 (build runs in the background, ~10 min)"
  ssh "${SSH_OPTS[@]}" "$1" 'cat > /tmp/sct-prepare.sh' < "$HERE/prepare_host.sh"
  # Detached, so the build survives this SSH session; progress in /var/log/sct-cloud-init.log.
  # shellcheck disable=SC2029
  ssh "${SSH_OPTS[@]}" "$1" \
    "sudo env SCT_ROLE=$2 SCT_REF=$SCT_REF SCT_PORT=$PORT setsid nohup bash /tmp/sct-prepare.sh < /dev/null > /dev/null 2>&1 &"
}
prepare "$SENDER" sender
[ -n "$RECEIVER" ] && prepare "$RECEIVER" receiver

# shellcheck disable=SC2029
ARCH="$(ssh "${SSH_OPTS[@]}" "$SENDER" uname -m)"
# shellcheck disable=SC2029
CPUS="$(ssh "${SSH_OPTS[@]}" "$SENDER" nproc)"
cat > "$STATE" <<STATE_EOF
PROVIDER=${PROVIDER:-own-hosts}
SIZE=${CPUS}cpu
ARCH=$ARCH
SCT_REF=$SCT_REF
SENDER_LOCATION=${SENDER_LOCATION:-unknown}
RECEIVER_LOCATION=${RECEIVER_LOCATION:-unknown}
SENDER_IP=$(addr_of "$SENDER")
RECEIVER_IP=$( [ -n "$RECEIVER" ] && addr_of "$RECEIVER" )
SSH_KEY=${SCT_SSH_KEY:-}
ADMIN=$(user_of "$SENDER")
PORT=$PORT
TEARDOWN="nothing to delete; stop or reset the hosts yourself"
STATE_EOF
echo "State in $STATE. Next: bash infra/linux/run_test.sh $( [ -n "$RECEIVER" ] && echo '[--campaign]' || echo '--campaign-only')"
