SQLX_OFFLINE := true
TEST_CONTAINER := hw-bot-test-postgres
COMPOSE := docker compose -f tools/docker-compose.yml

run:
	cargo run

check:
	SQLX_OFFLINE=$(SQLX_OFFLINE) cargo check

fmt:
	cargo fmt --check

fmt-fix:
	cargo fmt

lint:
	SQLX_OFFLINE=$(SQLX_OFFLINE) cargo clippy --all-targets -- -D warnings

lint-fix:
	SQLX_OFFLINE=$(SQLX_OFFLINE) cargo clippy --fix --all-targets --allow-dirty --allow-staged -- -D warnings

test:
	@trap 'docker rm -fv $(TEST_CONTAINER) >/dev/null 2>&1 || true' EXIT; \
		SQLX_OFFLINE=$(SQLX_OFFLINE) cargo test

test-lib:
	SQLX_OFFLINE=$(SQLX_OFFLINE) cargo test --lib

test-integration:
	SQLX_OFFLINE=$(SQLX_OFFLINE) cargo test --test integration

compose-up:
	$(COMPOSE) up -d

compose-down:
	$(COMPOSE) down --volumes --remove-orphans

.PHONY: run check fmt fmt-fix lint lint-fix test test-lib test-integration compose-up compose-down
