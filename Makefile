export SQLX_OFFLINE := true

COVERAGE_IGNORE := src/(test_support|domain/tests)/

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

coverage:
	cargo llvm-cov --locked --no-report
	cargo llvm-cov report --html --ignore-filename-regex '$(COVERAGE_IGNORE)'
	cargo llvm-cov report --json --summary-only --ignore-filename-regex '$(COVERAGE_IGNORE)' \
		--output-path target/llvm-cov/summary.json

.PHONY: all check fmt lint test audit coverage
