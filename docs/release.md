# Release checklist

Installer builds run only on `v*` tags (`.github/workflows/release.yml`).

## Already enforced in CI

Tag pushes **do not** run `ci.yml`. The Release workflow therefore starts with the same gates:

1. `cargo fmt --all -- --check`
2. `cargo clippy -p agent-doctor-core -p agent-doctor --all-targets -- -D warnings`
3. Desktop clippy on macOS

Build / upload jobs `need` those lint jobs. If fmt or clippy fails, **no** DMG/NSIS/CLI archives are produced (this is what stopped `v0.1.41`).

## Required before you tag

Do **not** push a version tag until all of the following are true:

1. Version bump is committed on `main` (`Cargo.toml`, `desktop/package.json`, `desktop/src-tauri/tauri.conf.json`, lockfiles).
2. Local preflight passes:

   ```bash
   ./scripts/check.sh release-preflight
   ```

   On macOS this includes desktop clippy. On Linux, set `AGENT_DOCTOR_CHECK_DESKTOP=1` only if GTK/WebKit deps are installed.
3. GitHub **CI on `main`** for that commit is green (fmt + clippy + tests). Wait for the Actions run; do not tag from a red `main`.
4. Then tag and push:

   ```bash
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin main
   git push origin vX.Y.Z
   ```

5. Watch the **Release** workflow until it finishes successfully and assets appear on the GitHub release page.

## Why this matters

Pushing a tag before CI/clippy is green wastes a full Release attempt and can leave an empty or incomplete GitHub release. The workflow gate is the last line of defense; **preflight + green `main` CI** is the first.
