# WAN campaign on Azure

Two VMs, one transfer path, then everything is deleted. Default path: **Frankfurt (`germanywestcentral`) → Johannesburg (`southafricanorth`)**, a real Europe–Africa route.

The scripts that run the test and the campaign are provider-neutral and live in [`infra/linux`](../linux/README.md); this folder only creates and deletes the Azure VMs.

## Before the first run

1. Azure CLI, `az login`, the subscription selected (`az account set -s …`).
2. Nothing else. The scripts create their own SSH key, resource group and firewall rules. SSH is open only to your current IP; the receiver's QUIC port (UDP 9410) only to the sender.

## Run

```bash
bash infra/azure/start_wan_test.sh          # ~1 min, then ~10 min build on the VMs
bash infra/linux/run_test.sh                # real WAN path, 3 runs
bash infra/linux/run_test.sh --campaign     # plus the netem campaign (~1–2 h)
bash infra/azure/teardown.sh                # ALWAYS
```

Other paths: `RECEIVER_LOCATION=eastus bash infra/azure/start_wan_test.sh` (compare with the AWS Frankfurt → Ashburn results), or `SENDER_LOCATION=southafricanorth RECEIVER_LOCATION=germanywestcentral` for the reverse direction.

VM size: the first of `VM_SIZES` offered in both regions wins. Default `Standard_D2ps_v6 Standard_D2ps_v5 Standard_D2as_v5`: ARM (Azure Cobalt / Ampere) first, x86 as a fallback. `environment.txt` records which one ran.

The campaign and the regional profiles are described in [`infra/linux/README.md`](../linux/README.md).

## Cost

Two `D2ps_v6` VMs cost cents per hour. The money goes to traffic leaving Azure and crossing continents: the default WAN test moves about 0.3 GB per run. The netem campaign stays on loopback and moves nothing across the network. `teardown.sh` deletes the whole resource group; check with `az group list --tag purpose=sct-wan-test -o table`.

Reporting: see [`infra/linux/README.md`](../linux/README.md#reporting).

## GitHub Actions

[`.github/workflows/wan-test-azure.yml`](../../.github/workflows/wan-test-azure.yml) runs the same scripts on `workflow_dispatch`. It signs in with OpenID Connect, so no Azure password or SSH key is stored in GitHub. Setup:

1. An app registration with a federated credential for this repository (`repo:SynapticFour/sc-transport:environment:azure-wan`), and the role *Contributor* on the test subscription only.
2. A GitHub environment `azure-wan` with the variables `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`.

Teardown runs even when a step fails.
