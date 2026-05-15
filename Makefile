SHELL := /bin/bash

PORT ?= 7272
SIZE_GB ?= 1
PROFILE ?= none
OUT_BASE ?= /tmp/sct-transfer-test
INTERFACE ?= lo
RATE_MBIT ?= 80
DELAY_MS ?= 110
LOSS ?= 0.008

# Serial sct-core integration binaries (matches .github/workflows/ci.yml job `test`).
SCT_CORE_ITESTS := completion_campaign completion_first_ab delta_transfer transfer_smoke \
	e2e_loopback fec_recovery prompt5_integration

.PHONY: ci ci-all ci-transport-integration ci-cli-daemon-smoke test-unit test-integration \
	test-integration-sct-core bench netem-test transfer-test profile-quic-good-large clean-results

ci:
	cargo fmt --all -- --check
	cargo clippy -p sct-core -p sct-proto --all-targets --all-features -- -D warnings
	cargo test --package sct-core --lib
	$(MAKE) test-integration
	SC_TRANSPORT_ARTIFACT_DIR=results/cf-check \
		cargo test --package sct-core --test completion_campaign completion_campaign_metrics -- --exact
	cargo test --package sct-proto
	@echo "ci: OK"

ci-transport-integration:
	bash scripts/ci-transport-integration.sh

ci-cli-daemon-smoke:
	bash scripts/ci-cli-daemon-smoke.sh

# Alias for `make ci` (both GitHub Actions jobs).
ci-all: ci

clean-results:
	find results -mindepth 1 ! -name README.md -exec rm -rf {} + 2>/dev/null || true
	rm -rf crates/sct-core/results/*
	@echo "clean-results: removed gitignored benchmark trees (kept results/README.md)"

test-unit:
	cargo test --package sct-core --lib
	cargo test -p sc-transport-core
	cargo test -p sc-transport-quic --features quic-streams
	cargo test -p sc-transport-sse
	cargo test -p sc-transport-datagrams --features quic-datagrams

test-integration-sct-core:
	@export RUST_TEST_THREADS=1; \
	for t in $(SCT_CORE_ITESTS); do \
		echo "==> sct-core --test $$t"; \
		features=; \
		[ "$$t" = e2e_loopback ] && features="--features test-hooks"; \
		cargo test --package sct-core --test $$t $$features -- --test-threads=1; \
	done

# All integration tests (no fmt/clippy/cf-check). Same binaries as CI integration steps.
test-integration: test-integration-sct-core ci-transport-integration ci-cli-daemon-smoke

bench:
	cargo run -p sct-bench -- synthetic --samples 5 --payload-mib 64

# good/large quic-stream matrix slice (target ≥400 eps); writes /tmp/streaming-matrix-summary.json
profile-quic-good-large:
	SC_TRANSPORT_ALLOW_INSECURE_QUIC=true \
	SC_STREAM_MATRIX_QUALITIES=good \
	SC_STREAM_MATRIX_PAYLOADS=large \
	SC_STREAM_MATRIX_REPEATS=5 \
	SC_TRANSPORT_ARTIFACT_DIR=/tmp \
	cargo test --release -p sc-transport --features transport-quic,transport-datagrams --test streaming_matrix streaming_transport_matrix_without_files -- --exact --nocapture
	@python3 -c "import json,sys; s=json.load(open('/tmp/streaming-matrix-summary.json')); \
q=[x for x in s['summaries'] if x['transport']=='quic-stream' and x['quality']=='good' and x['payload']=='large']; \
eps=q[0]['events_per_s_p50'] if q else 0; print(f'quic-stream good/large eps p50={eps:.1f}'); \
sys.exit(0 if eps>=400 else 1)"

netem-test:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "netem-test requires Linux"; \
		exit 1; \
	fi
	@if [ "$$(id -u)" -ne 0 ]; then \
		echo "netem-test requires root (tc netem)"; \
		exit 1; \
	fi
	@mkdir -p docs/RESULTS
	@echo "Running netem matrix on lo..."
	@cargo run -p sct-bench -- netem-matrix \
		--interface lo \
		--profile all \
		--sizes-mib 1,16,256,1024 \
		--output-json docs/RESULTS/$$(date +%F)-linux-netem-matrix.json
	@echo "netem-test completed"

transfer-test:
	@set -euo pipefail; \
	mkdir -p "$(OUT_BASE)/send" "$(OUT_BASE)/recv"; \
	SEND_FILE="$(OUT_BASE)/send/payload-$${SIZE_GB}g.bin"; \
	echo "Preparing $$SEND_FILE ($${SIZE_GB}GB)..."; \
	if [ ! -f "$$SEND_FILE" ]; then \
		if [ "$$(uname -s)" = "Darwin" ]; then \
			mkfile -n "$${SIZE_GB}g" "$$SEND_FILE"; \
		elif command -v fallocate >/dev/null 2>&1; then \
			fallocate -l "$${SIZE_GB}G" "$$SEND_FILE"; \
		else \
			dd if=/dev/zero of="$$SEND_FILE" bs=1M count=$$((SIZE_GB * 1024)) status=progress; \
		fi; \
	fi; \
	if [ "$(PROFILE)" = "toronto-auckland" ]; then \
		echo "Applying toronto-auckland profile..."; \
		if [ "$$(uname -s)" = "Darwin" ]; then \
			printf '%s\n%s\n' \
				"dummynet out quick proto udp from any to any port $(PORT) pipe 1" \
				"dummynet in  quick proto udp from any to any port $(PORT) pipe 1" \
				> /tmp/sct-netem-anchor.conf; \
			sudo dnctl pipe 1 config bw $(RATE_MBIT)Mbit/s delay $(DELAY_MS)ms plr $(LOSS); \
			sudo pfctl -a com.synapticfour.sct -f /tmp/sct-netem-anchor.conf; \
			sudo pfctl -e; \
			cleanup_profile() { \
				sudo pfctl -a com.synapticfour.sct -F rules || true; \
				sudo dnctl -q flush || true; \
				sudo pfctl -d || true; \
				rm -f /tmp/sct-netem-anchor.conf; \
			}; \
		elif [ "$$(uname -s)" = "Linux" ]; then \
			if [ "$$(id -u)" -ne 0 ]; then \
				echo "Linux shaping requires root (or run via sudo make ...)"; \
				exit 1; \
			fi; \
			tc qdisc del dev "$(INTERFACE)" root >/dev/null 2>&1 || true; \
			tc qdisc add dev "$(INTERFACE)" root netem delay $(DELAY_MS)ms 20ms loss $$(awk "BEGIN { print $(LOSS)*100 }")% rate $(RATE_MBIT)mbit; \
			cleanup_profile() { tc qdisc del dev "$(INTERFACE)" root >/dev/null 2>&1 || true; }; \
		else \
			echo "Unsupported OS for shaping profile"; \
			exit 1; \
		fi; \
		trap cleanup_profile EXIT; \
	fi; \
	echo "Starting receiver on port $(PORT)..."; \
	(cargo run -p sct-cli -- recv --port $(PORT) --output-dir "$(OUT_BASE)/recv" --once > "$(OUT_BASE)/recv.log" 2>&1) & \
	RECV_PID=$$!; \
	sleep 1; \
	echo "Sending $$SEND_FILE ..."; \
	time cargo run -p sct-cli -- send "$$SEND_FILE" sct://127.0.0.1:$(PORT) --compression none --quiet; \
	wait $$RECV_PID; \
	RECV_FILE="$$(ls -1 "$(OUT_BASE)/recv" | head -n 1)"; \
	if [ -n "$$RECV_FILE" ]; then \
		echo "Received file: $(OUT_BASE)/recv/$$RECV_FILE"; \
		shasum -a 256 "$$SEND_FILE" "$(OUT_BASE)/recv/$$RECV_FILE"; \
	else \
		echo "No received file found. Check $(OUT_BASE)/recv.log"; \
		exit 1; \
	fi
