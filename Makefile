build:
	docker compose build

up:
	docker compose up -d

down:
	docker compose down

logs:
	docker compose logs -f knowwhere

ps:
	docker compose ps

benchmark:
	docker compose exec knowwhere /app/scripts/benchmark.sh

shell:
	docker compose exec knowwhere /bin/bash

test:
	cargo test --lib

test-postgres:
	DATABASE_URL="postgresql://postgres:kw@localhost:5433/kw" cargo test --features postgres-storage

fmt:
	cargo fmt

clippy:
	cargo clippy --all-features

clean:
	cargo clean
	docker compose down -v

.PHONY: build up down logs ps benchmark shell test test-postgres fmt clippy clean
