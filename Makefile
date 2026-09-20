.PHONY: fmt fmt-check clippy-cli clippy-desktop lint test-cli build-cli frontend cli desktop check all hooks

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy-cli:
	cargo clippy -p agent-doctor-core -p agent-doctor --all-targets -- -D warnings

clippy-desktop:
	cargo clippy -p agent-doctor-desktop --all-targets -- -D warnings

# Hard local gate (same as git pre-commit / pre-push).
lint:
	./scripts/check.sh lint

hooks:
	./scripts/install-git-hooks.sh

test-cli:
	cargo test -p agent-doctor-core -p agent-doctor

build-cli:
	cargo build --release -p agent-doctor

frontend:
	cd desktop && npm ci && npm run build

cli: lint test-cli build-cli

desktop: clippy-desktop

check: cli frontend

all:
	./scripts/check.sh all
