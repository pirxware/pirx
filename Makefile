.PHONY: build build-release test bench check fmt audit ci setup clean \
        build-python fmt-python check-python test-python audit-python ci-python \
        sync-python ci-all clean-python

PYTHON_DIR := python

# ══════════════════════════════════════════════════════════════════════════════
# Rust
# ══════════════════════════════════════════════════════════════════════════════

# ── Build ─────────────────────────────────────────────────────────────────────

build:
	cargo build --locked

build-release:
	cargo build --locked --release

# ── Format & Lint ─────────────────────────────────────────────────────────────

fmt:
	cargo +nightly fmt --all
	cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged -- -D warnings

check:
	cargo +nightly fmt --all -- --check
	cargo clippy --locked --workspace --all-targets -- -D warnings

# ── Test ──────────────────────────────────────────────────────────────────────

test:
	cargo test --workspace --locked

# ── Bench ─────────────────────────────────────────────────────────────────────

bench:
	cargo bench --workspace

# ── Audit ─────────────────────────────────────────────────────────────────────

audit:
	cargo deny check

# ── CI gate (Rust only — run before pushing Rust changes) ─────────────────────

ci: check test audit
	@echo "  ✓ Rust checks passed"

# ══════════════════════════════════════════════════════════════════════════════
# Python
# ══════════════════════════════════════════════════════════════════════════════

# ── Sync (install/update venv + build extension) ─────────────────────────────

sync-py:
	cd $(PYTHON_DIR) && uv sync --all-extras
	cd $(PYTHON_DIR) && uv run maturin develop --release

# ── Build (extension only, no dependency sync) ───────────────────────────────

build-py:
	cd $(PYTHON_DIR) && uv run maturin develop --release

# ── Format & Lint ─────────────────────────────────────────────────────────────

fmt-py:
	cd $(PYTHON_DIR) && uv run ruff format pirx/ tests/
	cd $(PYTHON_DIR) && uv run ruff check --fix pirx/ tests/

check-py:
	cd $(PYTHON_DIR) && uv run ruff format --check pirx/ tests/
	cd $(PYTHON_DIR) && uv run ruff check pirx/ tests/
	cd $(PYTHON_DIR) && uv run mypy pirx/ --ignore-missing-imports

# ── Test ──────────────────────────────────────────────────────────────────────

test-py:
	cd $(PYTHON_DIR) && uv run pytest tests/ -v

test-py-tket:
	cd $(PYTHON_DIR) && uv run pytest tests/test_tket_adapter.py -v

test-py-qiskit:
	cd $(PYTHON_DIR) && uv run pytest tests/test_qiskit_adapter.py -v

test-py-qualtran:
	cd $(PYTHON_DIR) && uv run pytest tests/test_qualtran_adapter.py -v

# ── Audit ─────────────────────────────────────────────────────────────────────

audit-py:
	cd $(PYTHON_DIR) && uv run pip-audit

# ── CI gate (Python only — run before pushing Python changes) ─────────────────

ci-py: check-python test-python audit-python
	@echo "  ✓ Python checks passed"

# ══════════════════════════════════════════════════════════════════════════════
# Combined
# ══════════════════════════════════════════════════════════════════════════════

# ── Full CI gate (both Rust and Python — run before pushing) ──────────────────

ci-all: ci ci-python
	@echo "  ✓ All checks passed (Rust + Python)"

# ── Setup ─────────────────────────────────────────────────────────────────────

setup:
	git config core.hooksPath .githooks
	@echo "  ✓ Git hooks installed"
	@if command -v uv >/dev/null 2>&1; then \
		$(MAKE) sync-python; \
		echo "  ✓ Python environment synced"; \
	else \
		echo "  ⚠ uv not found — skipping Python setup (install: https://docs.astral.sh/uv/)"; \
	fi

# ── Clean ─────────────────────────────────────────────────────────────────────

clean:
	cargo clean

clean-python:
	rm -rf $(PYTHON_DIR)/.venv $(PYTHON_DIR)/target $(PYTHON_DIR)/*.egg-info

clean-all: clean clean-python
