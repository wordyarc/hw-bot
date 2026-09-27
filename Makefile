SQLX_OFFLINE := true
TEST_CONTAINER := hw-bot-test-postgres
COMPOSE := docker compose -f tools/deploy/compose.local.yml
PROD_COMPOSE := docker compose --env-file tools/deploy/.env.production -f tools/deploy/compose.yml

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

audit:
	cargo audit

lint-fix:
	SQLX_OFFLINE=$(SQLX_OFFLINE) cargo clippy --fix --all-targets --allow-dirty --allow-staged -- -D warnings

test:
	@trap 'docker rm -fv $(TEST_CONTAINER) >/dev/null 2>&1 || true' EXIT; \
		SQLX_OFFLINE=$(SQLX_OFFLINE) cargo test

test-lib:
	SQLX_OFFLINE=$(SQLX_OFFLINE) cargo test --lib

test-integration:
	SQLX_OFFLINE=$(SQLX_OFFLINE) cargo test --test integration

ci: fmt check lint test-lib test-integration

image:
	docker build --pull --tag hw-bot:local .

compose-up:
	$(COMPOSE) up -d

compose-down:
	$(COMPOSE) down --volumes --remove-orphans

# Production Compose

prod-bootstrap:
	$(PROD_COMPOSE) -f tools/deploy/compose.bootstrap.yml up -d --wait --wait-timeout 120

prod-up:
	$(PROD_COMPOSE) up -d --no-deps --force-recreate --wait --wait-timeout 120 app

prod-backup:
	sh tools/ops/backup.sh

.PHONY: run check fmt fmt-fix lint lint-fix test test-lib test-integration audit ci image compose-up compose-down prod-bootstrap prod-up prod-backup
