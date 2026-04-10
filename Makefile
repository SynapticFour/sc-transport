test-unit:
	cargo test

test-integration:
	cargo test --tests
	cargo test -p sct-core --test prompt5_integration

bench:
	cargo run -p sct-bench -- --samples 5 --payload-mib 64

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
	@./scripts/netem_runner.sh lo 0 datagram_delivery_no_loss | tee docs/RESULTS/$$(date +%F)-linux-netem-baseline.md
	@./scripts/netem_runner.sh lo 5 datagram_delivery_5pct_loss | tee docs/RESULTS/$$(date +%F)-linux-netem-5pct.md
	@./scripts/netem_runner.sh lo 20 datagram_delivery_20pct_loss | tee docs/RESULTS/$$(date +%F)-linux-netem-20pct.md
	@echo "netem-test completed"
