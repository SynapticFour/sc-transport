# WAN cloud test (optional)

The default CI gate does **not** need AWS, Fly.io, or any cloud account.

| What | Trigger | Cloud required |
|------|---------|----------------|
| **CI** (`ci.yml`) | push, PR, or manual *Run workflow* | No |
| CodeQL, secret scan, quality gate | push / schedule | No |
| **WAN Test (AWS EC2)** | manual or Monday 04:00 UTC | Yes (AWS) |
| **WAN Test (Azure)** | manual | Yes (Azure) |
| **WAN / netem on any Linux host** (`infra/linux`, Hetzner or own hosts) | local scripts only | Hetzner token or SSH to your hosts |
| **WAN Test (Fly.io)** | local script only | Yes (Fly.io, free tier) |

## Local WAN-shaped benchmarks (no cloud)

Use loopback + `tc/netem` on Linux:

```bash
make netem-test          # full matrix → docs/RESULTS/
make transfer-test PROFILE=good SIZE_GB=1
```

See [`NETWORK-EMULATION.md`](NETWORK-EMULATION.md).

## Fly.io (free, local script)

Cross-region test Frankfurt → Ashburn without AWS credits:

```bash
# See infra/flyio/README.md for one-time setup
bash scripts/wan_test_flyio.sh
```

## Any Linux host: the netem campaign and WAN paths

The test and the Linux `tc netem` campaign, which closes the open items in [`PRODUCTION-READINESS-CHECKLIST.md`](PRODUCTION-READINESS-CHECKLIST.md) section A, run on any Ubuntu 24.04 host with root. Hosts come from Hetzner Cloud (billed by the hour), Azure, or hosts you already have. See [`infra/linux/README.md`](../infra/linux/README.md).

```bash
bash infra/hetzner/start_wan_test.sh            # or infra/azure/start_wan_test.sh, or infra/linux/hosts.sh
bash infra/linux/run_test.sh --campaign-only    # or --campaign with a receiver
bash infra/hetzner/teardown.sh
```

Frankfurt → Johannesburg: Azure ([`infra/azure/README.md`](../infra/azure/README.md), locally or from GitHub Actions with OpenID Connect) or a host in Johannesburg with `infra/linux/hosts.sh`.

## AWS EC2 in GitHub Actions

1. Apply for [AWS Activate Credits](https://aws.amazon.com/startups/credits)
   (Founders package: $1,000).
2. Create EC2 key pair `sct-wan-key` and security group `sg-sct-wan` (SSH + port 9410)
   in `eu-central-1` and `us-east-1`. See [`infra/aws/README.md`](../infra/aws/README.md).
3. Add repository secrets (Settings → Secrets and variables → Actions):

   | Secret | Purpose |
   |--------|---------|
   | `AWS_ACCESS_KEY_ID` | IAM user with EC2 permissions |
   | `AWS_SECRET_ACCESS_KEY` | Matching secret key |
   | `SCT_WAN_SSH_PRIVATE_KEY` | Private key for `sct-wan-key` (SSH to sender VM) |

4. In GitHub: **Actions** → **WAN Test (AWS EC2 — Activate Credits)** → **Run workflow**.

Scheduled runs: Mondays 04:00 UTC. Teardown runs always (even on failure).

If AWS secrets are missing, the workflow fails at credential configuration — normal CI is unaffected.
