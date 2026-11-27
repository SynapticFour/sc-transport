#!/bin/bash
# Start one sender and one receiver VM for a WAN campaign on Azure.
# Default path: Frankfurt (germanywestcentral) -> Johannesburg (southafricanorth).
#
# Env (all optional):
#   SENDER_LOCATION    default germanywestcentral
#   RECEIVER_LOCATION  default southafricanorth
#   VM_SIZES           space-separated preference list; first size offered in BOTH
#                      locations wins. Default: ARM first, then x86.
#   SCT_REF            git ref the VMs build (default: current commit if pushed, else main)
#   SCT_STATE          state file (default /tmp/sct-wan.env); read by infra/linux/run_test.sh
#   SCT_AZ_SSH_KEY     private key path; generated if missing (default /tmp/sct-azure-wan-key)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SENDER_LOCATION="${SENDER_LOCATION:-germanywestcentral}"
RECEIVER_LOCATION="${RECEIVER_LOCATION:-southafricanorth}"
VM_SIZES="${VM_SIZES:-Standard_D2ps_v6 Standard_D2ps_v5 Standard_D2as_v5}"
STATE="${SCT_STATE:-${SCT_AZ_STATE:-/tmp/sct-wan.env}}"
SSH_KEY="${SCT_AZ_SSH_KEY:-/tmp/sct-azure-wan-key}"
ADMIN=sct
PORT=9410

if [ -z "${SCT_REF:-}" ]; then
  head="$(git -C "$HERE" rev-parse HEAD 2>/dev/null || true)"
  if [ -n "$head" ] && git -C "$HERE" branch -r --contains "$head" 2>/dev/null | grep -q .; then
    SCT_REF="$head"
  else
    SCT_REF=main
  fi
fi

command -v az >/dev/null || { echo "az CLI not found" >&2; exit 1; }
az account show >/dev/null || { echo "Run 'az login' first" >&2; exit 1; }

offered() { # size location
  az vm list-skus --location "$2" --size "$1" --resource-type virtualMachines \
    --query "[?name=='$1' && length(restrictions)==\`0\`] | length(@)" -o tsv 2>/dev/null
}
SIZE=""
for s in $VM_SIZES; do
  if [ "$(offered "$s" "$SENDER_LOCATION")" = "1" ] && [ "$(offered "$s" "$RECEIVER_LOCATION")" = "1" ]; then
    SIZE="$s"; break
  fi
done
[ -n "$SIZE" ] || { echo "None of [$VM_SIZES] is offered in both $SENDER_LOCATION and $RECEIVER_LOCATION" >&2; exit 1; }
case "$SIZE" in
  *p[sd]*_v*|*pls_v*) IMAGE="Canonical:ubuntu-24_04-lts:server-arm64:latest"; ARCH=arm64 ;;
  *) IMAGE="Canonical:ubuntu-24_04-lts:server:latest"; ARCH=amd64 ;;
esac

[ -f "$SSH_KEY" ] || ssh-keygen -q -t ed25519 -N "" -f "$SSH_KEY" -C sct-azure-wan
RG="sct-wan-$(date -u +%Y%m%d%H%M%S)"
MY_IP="$(curl -fsS https://api.ipify.org)"

echo "Resource group $RG, size $SIZE ($ARCH), ref $SCT_REF"
az group create -n "$RG" -l "$SENDER_LOCATION" --tags purpose=sct-wan-test >/dev/null

make_vm() { # name location role
  local data; data="$(mktemp)"
  { echo "#!/bin/bash"; echo "SCT_ROLE=$3"; echo "SCT_REF=$SCT_REF"; echo "SCT_PORT=$PORT";
    tail -n +2 "$HERE/../linux/prepare_host.sh"; } > "$data"
  az vm create -g "$RG" -n "$1" -l "$2" --image "$IMAGE" --size "$SIZE" \
    --admin-username "$ADMIN" --ssh-key-values "$SSH_KEY.pub" \
    --public-ip-sku Standard --nsg-rule NONE --custom-data "$data" \
    --tags purpose=sct-wan-test role="$3" \
    --query publicIpAddress -o tsv
  rm -f "$data"
}
SENDER_IP="$(make_vm sct-sender "$SENDER_LOCATION" sender)"
RECEIVER_IP="$(make_vm sct-receiver "$RECEIVER_LOCATION" receiver)"

# SSH only from this machine; QUIC (UDP) on the receiver only from the sender.
# `az vm create --nsg-rule NONE` leaves an empty NSG named <vm>NSG.
allow() { # vm rule-name priority protocol port source
  az network nsg rule create -g "$RG" --nsg-name "$1NSG" -n "$2" --priority "$3" \
    --direction Inbound --access Allow --protocol "$4" --destination-port-ranges "$5" \
    --source-address-prefixes "$6" >/dev/null
}
allow sct-sender   ssh-admin 1000 Tcp 22    "$MY_IP/32"
allow sct-receiver ssh-admin 1000 Tcp 22    "$MY_IP/32"
allow sct-receiver sct-quic  1010 Udp "$PORT" "$SENDER_IP/32"

cat > "$STATE" <<STATE_EOF
PROVIDER=azure
RG=$RG
SIZE=$SIZE
ARCH=$ARCH
SCT_REF=$SCT_REF
SENDER_LOCATION=$SENDER_LOCATION
RECEIVER_LOCATION=$RECEIVER_LOCATION
SENDER_IP=$SENDER_IP
RECEIVER_IP=$RECEIVER_IP
SSH_KEY=$SSH_KEY
ADMIN=$ADMIN
PORT=$PORT
TEARDOWN="bash infra/azure/teardown.sh"
STATE_EOF
echo "Sender   $SENDER_IP ($SENDER_LOCATION)"
echo "Receiver $RECEIVER_IP ($RECEIVER_LOCATION)"
echo "State in $STATE. Build takes ~10 minutes; then: bash infra/linux/run_test.sh [--campaign]"
echo "ALWAYS finish with: bash infra/azure/teardown.sh"
