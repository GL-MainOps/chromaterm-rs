# chromaterm-rs — common tasks. `make help` lists them.
BIN      := ct
VERSION  := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
DIST     := dist
BINDIR   ?= $(HOME)/.local/bin
TARGETS  := x86_64-unknown-linux-gnu x86_64-unknown-linux-musl \
            aarch64-unknown-linux-gnu aarch64-unknown-linux-musl

.PHONY: help build test lint fmt check bench link release release-gnu release-all release-small install clean

help: ## Show this help
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-14s %s\n", $$1, $$2}'

build: ## Debug build
	cargo build

test: ## Run all tests (both feature sets)
	cargo test
	cargo test --no-default-features

lint: ## rustfmt check + clippy (warnings are errors)
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo clippy --all-targets --no-default-features -- -D warnings

fmt: ## Format the code
	cargo fmt

check: lint test ## Full quality gate (run before committing)

bench: ## Criterion benchmarks (CT_BENCH_CONFIG=path adds your config)
	cargo bench --bench highlight

link: ## Build an optimized native binary and symlink it as $(BINDIR)/ct
	cargo build --release --locked
	@mkdir -p "$(BINDIR)"
	@src="$(CURDIR)/target/release/$(BIN)"; dest="$(BINDIR)/$(BIN)"; \
	if { [ -e "$$dest" ] || [ -L "$$dest" ]; } && [ "$$(readlink -f "$$dest")" != "$$(readlink -f "$$src")" ]; then \
		bak="$$dest.bak-$$(date +%Y%m%d-%H%M%S)"; mv "$$dest" "$$bak"; echo "moved previous $$dest to $$bak"; \
	fi; \
	ln -sfn "$$src" "$$dest"; echo "$$dest -> $$src"; "$$dest" --version

release: ## Static x86_64 musl binary → dist/
	ci/build-release.sh x86_64-unknown-linux-musl $(DIST)

release-gnu: ## Fast x86_64 glibc (≥ 2.28) binary → dist/ (needs python3 for cargo-zigbuild)
	ci/build-release.sh x86_64-unknown-linux-gnu $(DIST)

release-all: ## All release binaries (x86_64/aarch64 × gnu/musl) + SHA256SUMS → dist/
	@for t in $(TARGETS); do ci/build-release.sh $$t $(DIST) || exit 1; done
	ci/checksums.sh $(DIST)

release-small: ## Size-optimized x86_64 musl binary (opt-level=s, no YAML importer)
	cargo build --profile release-small --no-default-features --target x86_64-unknown-linux-musl
	@ls -l target/x86_64-unknown-linux-musl/release-small/$(BIN)

install: release ## Install the static musl binary to $(BINDIR) (a copy, not a link)
	install -Dm755 $(DIST)/$(BIN)-$(VERSION)-x86_64-linux-musl $(BINDIR)/$(BIN)

clean: ## Remove build artifacts
	cargo clean
	rm -rf $(DIST)
