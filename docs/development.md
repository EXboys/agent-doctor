# Development

## Local checks (match CI)

The workspace includes **CLI crates** and a **Tauri desktop crate**. On Linux, the desktop crate needs GTK/WebKit system libraries; CI therefore splits checks by platform.

### Model presets (Hermes)

```bash
agent-doctor profile init          # create ~/.config/agent-doctor/profiles.yaml
agent-doctor profile list
agent-doctor profile use work      # writes Hermes config + backup
agent-doctor config show hermes
```

Edit `~/.config/agent-doctor/profiles.yaml` to add your own presets. Example: [profiles.example.yaml](../examples/profiles.example.yaml).

### Quick commands

```bash
# Hard local gate (fmt + clippy) — also enforced by git hooks
make lint
# or
./scripts/check.sh lint

# Install / refresh git hooks (once per clone; release.sh also installs)
./scripts/install-git-hooks.sh
# or
make hooks

# Same as CI on any OS (CLI + frontend)
make check
# or
./scripts/check.sh cli && ./scripts/check.sh frontend

# Full local sweep on macOS (includes desktop Rust)
./scripts/check.sh all

# Format fix
make fmt
# or
./scripts/check.sh fmt
```

Git hooks (after `./scripts/install-git-hooks.sh`):

- **pre-commit** — if the commit touches Rust/Cargo, runs `./scripts/check.sh lint`
- **pre-push** — **every** push runs lint; pushing `v*` tags also requires release-preflight
- Emergency only: `AGENT_DOCTOR_SKIP_LINT=1` (do not use for real work)

### Cargo aliases

```bash
cargo check-cli   # clippy core + CLI
cargo test-cli    # test core + CLI
```

### Config files

| File | Purpose |
|------|---------|
| `rustfmt.toml` | rustfmt style |
| `clippy.toml` | clippy settings |
| `.cargo/config.toml` | cargo aliases |

## Linux: desktop Rust builds

If you see `Package 'glib-2.0' not found` when building `agent-doctor-desktop`:

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  libappindicator3-dev \
  librsvg2-dev \
  patchelf \
  pkg-config
```

Then:

```bash
AGENT_DOCTOR_CHECK_DESKTOP=1 ./scripts/check.sh desktop
```

## CI layout

| Job | Runner | Scope |
|-----|--------|-------|
| `rust-cli` | ubuntu-latest | fmt, clippy, test, build for `agent-doctor-core` + `agent-doctor` |
| `rust-desktop` | macos-latest | clippy for `agent-doctor-desktop` |
| `desktop-frontend` | ubuntu-latest | `npm ci && npm run build` |

Release builds install Linux GUI dependencies only in the desktop release job.
