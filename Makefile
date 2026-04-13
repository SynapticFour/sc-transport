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
