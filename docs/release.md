# Release checklist

Installer builds run only on `v*` tags (`.github/workflows/release.yml`).

## Local gate (required)

Detect fmt/clippy/test failures **on your machine** before anything hits GitHub Releases:

```bash
# one-time per clone
./scripts/install-git-hooks.sh

# preferred: all-in-one (preflight → tag → push)
./scripts/release.sh          # uses Cargo.toml version
./scripts/release.sh v0.1.42  # must match Cargo.toml
```

What this enforces:

1. `./scripts/check.sh release-preflight` runs `fmt --check`, clippy (`-D warnings`), and tests (+ desktop clippy on macOS).
2. A successful preflight writes `.git/agent-doctor-preflight.sha` for the current `HEAD`.
3. The **pre-push** hook blocks `refs/tags/v*` unless that stamp matches the tagged commit (otherwise it runs preflight again).
4. Emergency only: `AGENT_DOCTOR_SKIP_RELEASE_PREFLIGHT=1 git push origin vX.Y.Z`

Do **not** `git tag` + `git push --tags` without installing the hook or using `scripts/release.sh`.

## Already enforced online

Tag pushes **do not** run `ci.yml`. The Release workflow therefore starts with the same gates:

1. `cargo fmt --all -- --check`
2. `cargo clippy -p agent-doctor-core -p agent-doctor --all-targets -- -D warnings`
3. Desktop clippy on macOS

Build / upload jobs `need` those lint jobs. If fmt or clippy fails, **no** DMG/NSIS/CLI archives are produced.

## Version bump

Before `./scripts/release.sh`:

1. Bump and commit on `main`: `Cargo.toml`, `desktop/package.json`, `desktop/src-tauri/tauri.conf.json`, lockfiles.
2. Working tree clean; on `main`.
3. Run release script (local gate + tag + push).
4. Watch the **Release** workflow until assets appear.

## Why both local and CI

- **Local**: fail in seconds–minutes on the author machine; no empty GitHub release.
- **CI**: last line of defense if someone bypasses the hook.
