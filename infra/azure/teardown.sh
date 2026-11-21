#!/bin/bash
# Delete the whole resource group. ALWAYS run after a test.
set -euo pipefail
STATE="${SCT_AZ_STATE:-/tmp/sct-azure-wan.env}"
# shellcheck disable=SC1090
source "$STATE"
az group delete -n "$RG" --yes --no-wait
echo "Deleting $RG (runs in the background). Check: az group exists -n $RG"
