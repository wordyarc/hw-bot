export SQLX_OFFLINE := true

all:
	cargo build --locked --all-targets

check: fmt all lint test audit

fmt:
	cargo fmt --check

lint:
	cargo clippy --locked --all-targets -- -D warnings

audit:
	cargo audit

test:
	cargo test --locked

.PHONY: all check fmt lint test audit
