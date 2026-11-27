# WAN test and netem campaign on any Linux host

The test and the campaign need only Ubuntu 24.04 hosts with root (or passwordless sudo) and SSH. They do not depend on a cloud. Three ways to get the hosts:

| Way | Hosts | Cost | Start | Teardown |
|-----|-------|------|-------|----------|
| [Hetzner Cloud](../hetzner/README.md) | created and deleted per run, billed by the hour | cents per campaign | `bash infra/hetzner/start_wan_test.sh` | `bash infra/hetzner/teardown.sh` |
| [Azure](../azure/README.md) | created and deleted per run | cents per hour plus traffic | `bash infra/azure/start_wan_test.sh` | `bash infra/azure/teardown.sh` |
| Your own hosts (Google, Hostman, Vultr, Contabo, a lab server) | already there | what you pay anyway | `bash infra/linux/hosts.sh user@sender [user@receiver]` | nothing is created |

Every start script writes the same state file (`/tmp/sct-wan.env`, or `SCT_STATE`). Then, for all three:

```bash
bash infra/linux/run_test.sh                  # real WAN path, 3 runs (needs a receiver)
bash infra/linux/run_test.sh --campaign       # plus the netem campaign (~1–2 h)
bash infra/linux/run_test.sh --campaign-only  # the netem campaign alone, sender only
```

Results land in `wan-results/<UTC timestamp>/`:

| File | Content |
|------|---------|
| `environment.txt` | date, provider, locations, size, CPU architecture, commit, kernel, median TCP connect RTT |
| `wan-run-{1,2,3}/` | `wan_test.sh` over the real path: 1, 10, 100 MB, three independent runs |
| `campaign.tgz` | with `--campaign` or `--campaign-only`: the Linux netem campaign below |

## Files

| File | What it does |
|------|--------------|
| [`prepare_host.sh`](prepare_host.sh) | builds `sct` on the host; runs as cloud-init user data or over SSH |
| [`run_test.sh`](run_test.sh) | waits for the hosts, runs the WAN test and the campaign, collects results |
| [`run_campaign.sh`](run_campaign.sh) | the netem campaign, on the sender as root |
| [`profiles.txt`](profiles.txt) | simulated regional paths (assumptions) |
| [`hosts.sh`](hosts.sh) | prepares hosts you already have and writes the state file |

With your own hosts, open UDP 9410 on the receiver for the sender's address in the provider's firewall; `hosts.sh` does not touch firewalls.

## The netem campaign

Runs on the sender as root, on a real Linux kernel with `sch_netem`. It covers the open Linux items in section A of [`docs/PRODUCTION-READINESS-CHECKLIST.md`](../../docs/PRODUCTION-READINESS-CHECKLIST.md):

| Checklist item | What the campaign runs |
|----------------|------------------------|
| Repeat on a dedicated Linux host with tc netem | `sct-bench netem-matrix --profile all` (good, bad, very-bad) |
| Packet-loss integration tests with tc netem applied | `datagram_fallback_trigger` under 5 %, 15 %, 20 % kernel loss |
| Fallback at and above 15 % loss under real impairment | same, 15 % and 20 % |
| Latency and throughput benches with and without netem | `cargo bench --bench latency --bench throughput`, plain and shaped |
| Reproducibility across 3 independent runs | everything above, `SCT_CAMPAIGN_RUNS=3` (default) |

It stops before measuring anything if the kernel cannot load `sch_netem`. The campaign stays on loopback and moves nothing across the network, so one host is enough.

### Regional profiles: assumptions, not measurements

[`profiles.txt`](profiles.txt) adds loopback profiles for **Tunis**, **Accra** and **Johannesburg** to Frankfurt. The delay, loss and rate values are planning assumptions. Replace them with RIPE Atlas or partner-site measurements before quoting a regional result publicly, and say in the report which values were used.

## Reporting

Copy the numbers into a dated report under [`docs/RESULTS/`](../../docs/RESULTS/) using [`template.md`](../../docs/RESULTS/template.md), with the provider, commit, size, architecture and kernel from `environment.txt`. Only a dated report counts as a published result.
