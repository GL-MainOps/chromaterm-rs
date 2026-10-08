# chromaterm-rs — common tasks. `make help` lists them.
BIN      := ct
VERSION  := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TARGETS  := x86_64-unknown-linux-musl aarch64-unknown-linux-musl
DIST     := dist

.PHONY: help build test lint fmt check bench release release-all release-small install clean

help: ## Show this help
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-14s %s\n", $$1, $$2}'

build: ## Debug build
	cargo build

test: ## Run all tests
	cargo test

lint: ## rustfmt check + clippy (warnings are errors)
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

fmt: ## Format the code
	cargo fmt

check: lint test ## Full quality gate (run before committing)

bench: ## Criterion benchmarks (CT_BENCH_CONFIG=path adds your config)
	cargo bench --bench highlight

release: ## Static x86_64 musl binary → dist/
	cargo build --release --target x86_64-unknown-linux-musl
	@mkdir -p $(DIST)
	cp target/x86_64-unknown-linux-musl/release/$(BIN) $(DIST)/$(BIN)-$(VERSION)-x86_64-linux-musl
	@cd $(DIST) && sha256sum $(BIN)-$(VERSION)-x86_64-linux-musl > $(BIN)-$(VERSION)-x86_64-linux-musl.sha256
	@ls -l $(DIST)/$(BIN)-$(VERSION)-x86_64-linux-musl

release-all: ## Static musl binaries for x86_64 and aarch64 → dist/
	@mkdir -p $(DIST)
	@for t in $(TARGETS); do \
		cargo build --release --target $$t || exit 1; \
		arch=$${t%%-*}; out=$(DIST)/$(BIN)-$(VERSION)-$$arch-linux-musl; \
		cp target/$$t/release/$(BIN) $$out; \
		(cd $(DIST) && sha256sum $$(basename $$out) > $$(basename $$out).sha256); \
	done
	@ls -l $(DIST)

release-small: ## Size-optimized x86_64 musl binary (opt-level=s, no YAML importer)
	cargo build --profile release-small --no-default-features --target x86_64-unknown-linux-musl
	@ls -l target/x86_64-unknown-linux-musl/release-small/$(BIN)

install: release ## Install the static binary to ~/.local/bin
	install -Dm755 $(DIST)/$(BIN)-$(VERSION)-x86_64-linux-musl $(HOME)/.local/bin/$(BIN)

clean: ## Remove build artifacts
	cargo clean
	rm -rf $(DIST)
