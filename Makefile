CARGO ?= cargo
# Extra arguments for the smoke test, e.g. make smoke SMOKE_ARGS='--mic "Headset Microphone (...)" --secs 10'
SMOKE_ARGS ?=

.DEFAULT_GOAL := help
.PHONY: help build release installer run run-release test test-all fmt fmt-check lint check smoke clean

help: ## List targets
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-12s %s\n", $$1, $$2}'

build: ## Debug build
	$(CARGO) build

release: ## Optimized build -> target/release/vsoundboard.exe
	$(CARGO) build --release

installer: release ## Build dist/vSoundboard-Setup-<version>.exe (needs Inno Setup 7's iscc on PATH)
	iscc installer/vsoundboard.iss

run: ## Run the app (debug, with a console for logs)
	$(CARGO) run

run-release: ## Run the optimized app
	$(CARGO) run --release

test: ## Unit tests
	$(CARGO) test --lib --bins

test-all: ## Unit tests plus the ignored ones (needs target/fixtures audio files and real devices)
	$(CARGO) test --lib --bins -- --include-ignored

fmt: ## Format the code
	$(CARGO) fmt

fmt-check: ## Fail if the code isn't formatted
	$(CARGO) fmt -- --check

lint: ## Clippy with warnings as errors
	$(CARGO) clippy --all-targets -- -D warnings

check: fmt-check lint test ## Everything CI would run

smoke: ## Play a test tone through VB-Cable and report what arrived (SMOKE_ARGS=...)
	$(CARGO) run --example smoke -- $(SMOKE_ARGS)

clean: ## Remove build output
	$(CARGO) clean
