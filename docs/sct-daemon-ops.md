# sct-daemon Operations Runbook

## Scope

This runbook covers operational behavior for `sct-daemon` transfer execution and restart recovery.

## Runtime files

- Snapshot: `.sct-daemon/transfers.json`
- Event log (WAL-style): `.sct-daemon/events.jsonl`

The daemon replays both on startup to reconstruct transfer state.

## Transfer lifecycle

- `queued`: accepted and waiting for worker slot
- `active`: worker currently executing
- `completed`: terminal success
- `failed`: terminal failure with error message
- `cancelled`: terminal cancellation

## Recovery semantics

- If the daemon restarts while a transfer is `active`, it is recovered as `queued`.
- `cancelled` transfers are not requeued on startup.
- Priority updates requeue the transfer with the new priority.

## Incident handling

- TOFU mismatch:
  - symptom: connection errors containing `host key mismatch`
  - action: inspect and clean stale entries in `~/.sct/known_hosts`
- Interrupted receives:
  - symptom: `*.part` and `*.state.json` files in output directories
  - action: leave files in place for resume; remove only if transfer is intentionally abandoned
- Crash recovery validation:
  - restart daemon
  - check `/v1/transfers` for recovered `queued` items
  - monitor `/v1/metrics` for queue drain

## Suggested verification commands

```bash
cargo test -p sct-daemon
cargo check -p sct-core -p sct-cli -p sct-daemon
python -m pytest -q ./sct-rucio/tests
```
