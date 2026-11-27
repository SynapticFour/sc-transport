#!/bin/bash
# Delete the servers, firewall and SSH key of the last Hetzner run. ALWAYS run after a test.
set -euo pipefail
STATE="${SCT_STATE:-/tmp/sct-wan.env}"
# shellcheck disable=SC1090
source "$STATE"
: "${HCLOUD_RUN:?state file is not from infra/hetzner/start_wan_test.sh}"
for name in $(hcloud server list -l run="$HCLOUD_RUN" -o noheader -o columns=name); do
  hcloud server delete "$name"
done
hcloud firewall delete "$HCLOUD_RUN" || true
hcloud ssh-key delete "$HCLOUD_RUN" || true
echo "Deleted run $HCLOUD_RUN. Check: hcloud server list -l purpose=sct-wan-test"
