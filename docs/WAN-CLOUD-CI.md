# WAN cloud test (optional, manual)

The default CI gate does **not** need AWS, Oracle, or any cloud account.

| What | Trigger | Cloud required |
|------|---------|----------------|
| **CI** (`ci.yml`) | push, PR, or manual *Run workflow* | No |
| CodeQL, secret scan, quality gate | push / schedule | No |
| **WAN Test (Oracle Always Free)** | manual only | Yes (OCI) |

## Local WAN-shaped benchmarks (no cloud)

Use loopback + `tc/netem` on Linux:

```bash
make netem-test          # full matrix → docs/RESULTS/
make transfer-test PROFILE=good SIZE_GB=1
```

See [`NETWORK-EMULATION.md`](NETWORK-EMULATION.md).

## When you want a real cross-region WAN run in GitHub Actions

1. Create an [Oracle Cloud](https://www.oracle.com/cloud/free/) Always Free account (or reuse an existing one).
2. Add repository secrets (Settings → Secrets and variables → Actions):

   | Secret | Purpose |
   |--------|---------|
   | `OCI_TENANCY_OCID` | Tenancy OCID |
   | `OCI_USER_OCID` | API user OCID |
   | `OCI_FINGERPRINT` | API key fingerprint |
   | `OCI_PRIVATE_KEY` | PEM private key (API signing) |
   | `OCI_COMPARTMENT_OCID` | Compartment for VMs |
   | `OCI_SSH_PUBLIC_KEY` | SSH public key installed on VMs |
   | `OCI_SSH_PRIVATE_KEY` | Matching SSH private key for Actions |

3. In GitHub: **Actions** → **WAN Test (Oracle Always Free)** → **Run workflow**.

There is **no cron schedule**; nothing runs until you start it.

If secrets are missing, the workflow finishes successfully after `readiness` (provision job skipped; no Terraform, no cost).

## AWS

An experimental AWS EC2 workflow existed only in a local branch; **main** uses Oracle (`infra/oracle/`). AWS is not required.
