# Hetzner Cloud: netem campaign and WAN paths, billed by the hour

The cheapest way to run the Linux netem campaign: one server for an hour or two, then deleted. Hetzner has no location in Africa; for Europe–Africa paths use [Azure](../azure/README.md) or your own host in Johannesburg with [`infra/linux/hosts.sh`](../linux/hosts.sh).

## Before the first run

1. A Hetzner Cloud project and an API token with read/write rights.
2. The [hcloud CLI](https://github.com/hetznercloud/cli) and `hcloud context create sct-wan` (asks for the token).

The scripts create their own SSH key, firewall and servers, all labelled `purpose=sct-wan-test`. SSH is open only to your current IP; the receiver's QUIC port (UDP 9410) only to the sender.

## Run

```bash
bash infra/hetzner/start_wan_test.sh            # one ARM sender in fsn1, ~10 min build
bash infra/linux/run_test.sh --campaign-only    # the netem campaign (~1–2 h)
bash infra/hetzner/teardown.sh                  # ALWAYS
```

A WAN path between two Hetzner locations: `RECEIVER_LOCATION=ash bash infra/hetzner/start_wan_test.sh`, then `bash infra/linux/run_test.sh --campaign`.

Defaults: sender `cax21` (ARM, 4 vCPU, 8 GB) in `fsn1`; receiver `cpx21` (x86), because ARM types are not offered in every location. Override with `HCLOUD_SENDER_TYPE`, `HCLOUD_RECEIVER_TYPE`, `SENDER_LOCATION`. Check current types and prices with `hcloud server-type list` before a run.

Status: written against the hcloud CLI documentation, not yet run against a live project. The first run should be watched, and `hcloud server list -l purpose=sct-wan-test` checked after the teardown.
