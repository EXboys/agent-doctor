# Contributing

Agent Doctor is in early bootstrap. Before opening large PRs, please open an issue describing:

- Which runtime adapter (OpenClaw, Hermes, Claude Code, Codex, …)
- Whether the change is local-only or requires a control-plane API

## Layout

- `crates/agent-doctor-core/` — shared discovery, doctor, company profile logic
- `cli/` — `agent-doctor` binary (Rust)
- `desktop/` — Tauri menubar app (Rust + TypeScript UI)
- `adapters/` — adapter contract docs; implementations live in `agent-doctor-core`
- `scripts/check.sh` — local fmt/clippy/test; run `./scripts/check.sh lint` (or `make lint`) before push; `./scripts/check.sh site` checks the marketing site
- `scripts/install-git-hooks.sh` — installs hard pre-commit / pre-push fmt+clippy gates (required once per clone)
- `docs/` — user docs; optional enterprise integration in `enterprise.md`

Before touching the core flows (chat turn, desktop↔phone channel, config and keys, install, the site), read [docs/zh-CN/core-flows.md](docs/zh-CN/core-flows.md) — its invariants come from real regressions and name the tests that guard each one. Windows subprocess creation flags, the environment we hand to external agent runtimes, and pinned runtime version bumps have their own page: [docs/zh-CN/windows-spawn-and-runtime-env.md](docs/zh-CN/windows-spawn-and-runtime-env.md).

## Code of conduct

Be respectful. Security issues: please report privately via GitHub Security Advisories on this repository.
