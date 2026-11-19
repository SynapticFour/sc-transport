# WAN campaign on Azure

Two VMs, one transfer path, then everything is deleted. Default path: **Frankfurt (`germanywestcentral`) → Johannesburg (`southafricanorth`)**, a real Europe–Africa route.

What one run produces, in `wan-results/<UTC timestamp>/`:

| File | Content |
|------|---------|
| `environment.txt` | date, VM size, CPU architecture, commit on both VMs, kernel, median TCP connect RTT |
| `wan-run-{1,2,3}/` | `wan_test.sh` over the real path: 1, 10, 100 MB, three independent runs |
| `campaign.tgz` | with `--campaign` only: the Linux netem campaign below |

## Before the first run

1. Azure CLI, `az login`, the subscription selected (`az account set -s …`).
2. Nothing else. The scripts create their own SSH key, resource group and firewall rules. SSH is open only to your current IP; the receiver's QUIC port (UDP 9410) only to the sender.

## Run

```bash
bash infra/azure/start_wan_test.sh          # ~1 min, then ~10 min build on the VMs
bash infra/azure/run_test.sh                # real WAN path, 3 runs
bash infra/azure/run_test.sh --campaign     # plus the netem campaign (~1–2 h)
bash infra/azure/teardown.sh                # ALWAYS
```

Other paths: `RECEIVER_LOCATION=eastus bash infra/azure/start_wan_test.sh` (compare with the AWS Frankfurt → Ashburn results), or `SENDER_LOCATION=southafricanorth RECEIVER_LOCATION=germanywestcentral` for the reverse direction.

VM size: the first of `VM_SIZES` offered in both regions wins. Default `Standard_D2ps_v6 Standard_D2ps_v5 Standard_D2as_v5`: ARM (Azure Cobalt / Ampere) first, x86 as a fallback. `environment.txt` records which one ran.

## The netem campaign (`--campaign`)

Runs on the sender VM as root, on a real Linux kernel with `sch_netem`. It covers the open Linux items in section A of [`docs/PRODUCTION-READINESS-CHECKLIST.md`](../../docs/PRODUCTION-READINESS-CHECKLIST.md):

| Checklist item | What the campaign runs |
|----------------|------------------------|
| Repeat on a dedicated Linux host with tc netem | `sct-bench netem-matrix --profile all` (good, bad, very-bad) |
| Packet-loss integration tests with tc netem applied | `datagram_fallback_trigger` under 5 %, 15 %, 20 % kernel loss |
| Fallback at and above 15 % loss under real impairment | same, 15 % and 20 % |
| Latency and throughput benches with and without netem | `cargo bench --bench latency --bench throughput`, plain and shaped |
| Reproducibility across 3 independent runs | everything above, `SCT_CAMPAIGN_RUNS=3` (default) |

It stops before measuring anything if the kernel cannot load `sch_netem`.

### Regional profiles: assumptions, not measurements

[`profiles.txt`](profiles.txt) adds loopback profiles for **Tunis**, **Accra** and **Johannesburg** to Frankfurt. Azure has no region in Tunisia or Ghana, so these paths are simulated. The delay, loss and rate values are planning assumptions. Replace them with RIPE Atlas or partner-site measurements before quoting a regional result publicly, and say in the report which values were used.

## Cost

Two `D2ps_v6` VMs cost cents per hour. The money goes to traffic leaving Azure and crossing continents: the default WAN test moves about 0.3 GB per run. The netem campaign stays on loopback and moves nothing across the network. `teardown.sh` deletes the whole resource group; check with `az group list --tag purpose=sct-wan-test -o table`.

## Reporting

Copy the numbers into a dated report under [`docs/RESULTS/`](../../docs/RESULTS/) using [`template.md`](../../docs/RESULTS/template.md), with the commit, VM size, architecture and kernel from `environment.txt`. Only a dated report counts as a published result.

## GitHub Actions

[`.github/workflows/wan-test-azure.yml`](../../.github/workflows/wan-test-azure.yml) runs the same scripts on `workflow_dispatch`. It signs in with OpenID Connect, so no Azure password or SSH key is stored in GitHub. Setup:

1. An app registration with a federated credential for this repository (`repo:SynapticFour/sc-transport:environment:azure-wan`), and the role *Contributor* on the test subscription only.
2. A GitHub environment `azure-wan` with the variables `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`.

Teardown runs even when a step fails.
