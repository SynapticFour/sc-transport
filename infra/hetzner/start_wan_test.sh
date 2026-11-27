#!/bin/bash
# Start Hetzner Cloud servers for the netem campaign and, optionally, a WAN path.
# Billed by the hour; delete them with infra/hetzner/teardown.sh right after the run.
#
# Default: one ARM sender in Falkenstein (fsn1) for the netem campaign alone:
#   bash infra/hetzner/start_wan_test.sh && bash infra/linux/run_test.sh --campaign-only
# With RECEIVER_LOCATION (e.g. ash, hil, hel1) a receiver is created too, for a WAN test.
#
# Env (all optional):
#   SENDER_LOCATION        default fsn1
#   RECEIVER_LOCATION      unset = no receiver
#   HCLOUD_SENDER_TYPE     default cax21 (ARM, 4 vCPU, 8 GB)
#   HCLOUD_RECEIVER_TYPE   default cpx21 (x86; ARM types are not offered in every location)
#   SCT_REF, SCT_STATE (default /tmp/sct-wan.env), SCT_SSH_KEY (default /tmp/sct-hetzner-key)
#
# Needs the hcloud CLI with a project token (`hcloud context create sct-wan`).
# Written against the hcloud CLI documentation; not yet run against a live project.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
LINUX="$HERE/../linux"
SENDER_LOCATION="${SENDER_LOCATION:-fsn1}"
RECEIVER_LOCATION="${RECEIVER_LOCATION:-}"
SENDER_TYPE="${HCLOUD_SENDER_TYPE:-cax21}"
RECEIVER_TYPE="${HCLOUD_RECEIVER_TYPE:-cpx21}"
STATE="${SCT_STATE:-/tmp/sct-wan.env}"
SSH_KEY="${SCT_SSH_KEY:-/tmp/sct-hetzner-key}"
PORT=9410
TAG="sct-wan-$(date -u +%Y%m%d%H%M%S)"

if [ -z "${SCT_REF:-}" ]; then
  head="$(git -C "$HERE" rev-parse HEAD 2>/dev/null || true)"
  if [ -n "$head" ] && git -C "$HERE" branch -r --contains "$head" 2>/dev/null | grep -q .; then
    SCT_REF="$head"
  else
    SCT_REF=main
  fi
fi

command -v hcloud >/dev/null || { echo "hcloud CLI not found" >&2; exit 1; }
hcloud server list >/dev/null || { echo "No hcloud context; run 'hcloud context create'" >&2; exit 1; }

[ -f "$SSH_KEY" ] || ssh-keygen -q -t ed25519 -N "" -f "$SSH_KEY" -C sct-hetzner-wan
MY_IP="$(curl -fsS https://api.ipify.org)"
hcloud ssh-key create --name "$TAG" --public-key-from-file "$SSH_KEY.pub" --label purpose=sct-wan-test >/dev/null

# SSH only from this machine. The QUIC rule for the receiver is added once the sender has an IP.
RULES="$(mktemp)"
printf '[{"direction":"in","protocol":"tcp","port":"22","source_ips":["%s/32"]}]\n' "$MY_IP" > "$RULES"
hcloud firewall create --name "$TAG" --rules-file "$RULES" --label purpose=sct-wan-test >/dev/null
rm -f "$RULES"

make_server() { # name type location role
  local data; data="$(mktemp)"
  { echo "#!/bin/bash"; echo "SCT_ROLE=$4"; echo "SCT_REF=$SCT_REF"; echo "SCT_PORT=$PORT";
    tail -n +2 "$LINUX/prepare_host.sh"; } > "$data"
  hcloud server create --name "$1" --type "$2" --image ubuntu-24.04 --location "$3" \
    --ssh-key "$TAG" --firewall "$TAG" --user-data-from-file "$data" \
    --label purpose=sct-wan-test --label run="$TAG" >/dev/null
  rm -f "$data"
  hcloud server ip "$1"
}
SENDER_IP="$(make_server "$TAG-sender" "$SENDER_TYPE" "$SENDER_LOCATION" sender)"
RECEIVER_IP=""
if [ -n "$RECEIVER_LOCATION" ]; then
  RECEIVER_IP="$(make_server "$TAG-receiver" "$RECEIVER_TYPE" "$RECEIVER_LOCATION" receiver)"
  hcloud firewall add-rule "$TAG" --direction in --protocol udp --port "$PORT" \
    --source-ips "$SENDER_IP/32" >/dev/null
fi
case "$SENDER_TYPE" in cax*) ARCH=arm64 ;; *) ARCH=amd64 ;; esac

cat > "$STATE" <<STATE_EOF
PROVIDER=hetzner
HCLOUD_RUN=$TAG
SIZE=$SENDER_TYPE
ARCH=$ARCH
SCT_REF=$SCT_REF
SENDER_LOCATION=$SENDER_LOCATION
RECEIVER_LOCATION=$RECEIVER_LOCATION
SENDER_IP=$SENDER_IP
RECEIVER_IP=$RECEIVER_IP
SSH_KEY=$SSH_KEY
ADMIN=root
PORT=$PORT
TEARDOWN="bash infra/hetzner/teardown.sh"
STATE_EOF
echo "Sender   $SENDER_IP ($SENDER_LOCATION, $SENDER_TYPE)"
[ -n "$RECEIVER_IP" ] && echo "Receiver $RECEIVER_IP ($RECEIVER_LOCATION, $RECEIVER_TYPE)"
echo "Build takes ~10 minutes. Then: bash infra/linux/run_test.sh $( [ -n "$RECEIVER_IP" ] && echo '[--campaign]' || echo '--campaign-only')"
echo "ALWAYS finish with: bash infra/hetzner/teardown.sh"
