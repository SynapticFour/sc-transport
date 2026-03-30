# Production Readiness Checklist

This checklist is the final gate for declaring `sc-transport` fully ready for
production use in the Synaptic Core stack.

## Exit Criteria

- All items in sections A-D are checked.
- All mandatory artifacts are present in `docs/RESULTS/`.
- No unresolved Sev-1/Sev-2 transport issues remain open.

## A) Linux Netem Measurement Campaign (Required)

- [ ] Run packet-loss integration tests on Linux host with root privileges.
- [ ] Run latency and throughput benches on Linux with and without `tc netem`.
- [ ] Validate fallback behavior at and above threshold (`> 15%` loss).
- [ ] Confirm test reproducibility across at least 3 independent runs.

### Commands

- `cargo test --workspace --all-features`
- `cargo bench --bench throughput -- --quick`
- `cargo bench --bench latency -- --quick`
- `cargo bench --features transport-datagrams --bench packet_loss -- --quick`
- `scripts/netem_runner.sh lo 20 datagram_fallback_trigger`

### Required Artifacts

- [ ] `docs/RESULTS/<date>-linux-netem-baseline.md`
- [ ] `docs/RESULTS/<date>-linux-netem-5pct.md`
- [ ] `docs/RESULTS/<date>-linux-netem-20pct.md`
- [ ] Raw benchmark JSON/exports attached or linked from each report.

## B) QUIC Primary Path Validation (Required)

- [ ] QUIC stream path validated as primary (not fallback-only behavior).
- [ ] QUIC datagram loopback/path validated under `quic-datagrams` feature.
- [ ] MTU/truncation behavior verified with oversized event payloads.
- [ ] Reconnect/session behavior validated for repeated subscriptions.

### Required Evidence

- [ ] Passing test logs for QUIC stream/datagram feature builds.
- [ ] At least one documented failure-mode run and fallback recovery result.
- [ ] Metrics snapshot showing `fallback_count`, `events_dropped`, `events_sent`.

## C) Transparency Contract Sign-off (Required)

- [ ] `transport_transparency` test passes consistently in CI and local runs.
- [ ] Final workflow state equivalence confirmed across SSE/QUIC stream/datagram.
- [ ] Datagram subset semantics documented and acknowledged by stakeholders.
- [ ] No client-visible contract differences for final workflow state.

### Required Artifacts

- [ ] Updated `docs/DATAGRAM-SEMANTICS.md` (kept in sync with implementation).
- [ ] Updated `docs/FALLBACK-BEHAVIOR.md` with observed runtime behavior.

## D) Operational Readiness (Required)

- [ ] CI pipelines green for stable jobs (`ci.yml`).
- [ ] Experimental jobs run and archive results without gating merges.
- [ ] Regression comparison script produces actionable outputs.
- [ ] Automatic issue creation works for benchmark regressions in experimental CI.

### Required Artifacts

- [ ] One successful `experimental.yml` run with uploaded benchmark artifact.
- [ ] One regression simulation run producing a GitHub issue.

## Release Decision Record

Before release, create a final decision record in:

- [ ] `docs/RESULTS/<date>-production-readiness-decision.md`

Record must include:

- Scope/version being approved.
- Summary table of pass/fail for sections A-D.
- Known risks accepted by the team.
- Go/No-Go decision and approver names.
