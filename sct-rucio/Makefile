.PHONY: test test-unit test-integration build-docker up down lint

test-unit:
	python3 -m pip install -q -r requirements-dev.txt
	PYTHONPATH=. python3 -m pytest tests/test_plugin.py tests/test_rse_protocol.py -v

test-integration: up
	docker compose -f docker-compose.yml run --rm test-runner pytest tests/test_integration.py -v
	$(MAKE) down

up:
	docker compose -f docker-compose.yml up -d --build

down:
	docker compose -f docker-compose.yml down -v

lint:
	python3 -m pip install ruff -q && python3 -m ruff check sct_rucio/ tests/

build-daemon:
	cd .. && cargo build --release -p sct-daemon -p sct-cli

test: test-unit
