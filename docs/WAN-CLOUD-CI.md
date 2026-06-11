# WAN cloud test (optional)

The default CI gate does **not** need AWS, Fly.io, or any cloud account.

| What | Trigger | Cloud required |
|------|---------|----------------|
| **CI** (`ci.yml`) | push, PR, or manual *Run workflow* | No |
| CodeQL, secret scan, quality gate | push / schedule | No |
| **WAN Test (AWS EC2)** | manual or Monday 04:00 UTC | Yes (AWS) |
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
