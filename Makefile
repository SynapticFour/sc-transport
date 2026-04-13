SHELL := /bin/bash

PORT ?= 7272
SIZE_GB ?= 1
PROFILE ?= none
OUT_BASE ?= /tmp/sct-transfer-test
INTERFACE ?= lo
RATE_MBIT ?= 80
DELAY_MS ?= 110
LOSS ?= 0.008

test-unit:
	cargo test

test-integration:
	cargo test --tests
	cargo test -p sct-core --test prompt5_integration

bench:
	cargo run -p sct-bench -- synthetic --samples 5 --payload-mib 64

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
