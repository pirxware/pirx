.PHONY: build build-release test check fmt audit ci setup clean

# ── Build ─────────────────────────────────────────────────────────────────────

build:
	cargo build --locked

build-release:
	cargo build --locked --release

# ── Format & Lint ─────────────────────────────────────────────────────────────

fmt:
	cargo fmt --all
	cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged -- -D warnings

check:
	cargo fmt --all -- --check
	cargo clippy --locked --workspace --all-targets -- -D warnings

# ── Test ──────────────────────────────────────────────────────────────────────

test:
	cargo test --workspace --locked

# ── Audit ─────────────────────────────────────────────────────────────────────

audit:
	cargo deny check

# ── CI gate (run before pushing) ──────────────────────────────────────────────

ci: check test audit
	@echo "  ✓ All checks passed"

# ── Setup ─────────────────────────────────────────────────────────────────────

setup:
	git config core.hooksPath .githooks
	@echo "  ✓ Git hooks installed"

# ── Clean ─────────────────────────────────────────────────────────────────────

clean:
	cargo clean
