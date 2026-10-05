# Release checklist

Installer builds run only on `v*` tags (`.github/workflows/release.yml`).

## Local gate (required)

Detect fmt/clippy failures **on your machine** before anything hits GitHub:

```bash
# one-time per clone (installs pre-commit + pre-push)
./scripts/install-git-hooks.sh

# preferred: all-in-one (preflight → tag → push)
./scripts/release.sh          # uses Cargo.toml version
./scripts/release.sh v0.1.42  # must match Cargo.toml
```

What this enforces:

1. **Every commit** that touches Rust runs `./scripts/check.sh lint` (`fmt --check` + clippy `-D warnings`) via **pre-commit**.
2. **Every push** (branch or tag) runs the same lint via **pre-push**.
3. `./scripts/check.sh release-preflight` runs lint + tests (+ desktop clippy on macOS) and writes `.git/agent-doctor-preflight.sha`.
4. Pushing `refs/tags/v*` additionally requires that stamp (or runs preflight again).
5. Emergency only:
   - `AGENT_DOCTOR_SKIP_LINT=1 git push …`
   - `AGENT_DOCTOR_SKIP_RELEASE_PREFLIGHT=1 git push origin vX.Y.Z`

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

## Stable and beta channels

The download page keeps two versions side by side. The channel comes from the version string:

| Version in manifests | Tag | Channel | GitHub Release | OSS download manifest | Updater `latest.json` |
| --- | --- | --- | --- | --- | --- |
| `0.1.54` | `v0.1.54` | stable | normal (becomes `releases/latest`) | `<prefix>/download.json` | updated |
| `0.1.54-beta.1` | `v0.1.54-beta.1` | beta | prerelease | `<prefix>/beta/download.json` | **not touched** |

`<prefix>` is `desktop/` (personal) or `desktop-team/` (team).

```bash
# beta: set all manifests to 0.1.54-beta.1, commit, then
./scripts/release.sh
# stable after testing: set manifests to 0.1.54, commit, then
./scripts/release.sh
```

Rules:

- Only `X.Y.Z` and `X.Y.Z-beta.N` are accepted by `release.sh`.
- Beta installs do not auto-update to newer betas. They update to the next stable release (`0.1.54` > `0.1.54-beta.N`). Testers install each new beta from the download page.
- Beta Windows builds ship NSIS `setup.exe` only (WiX MSI rejects `-beta.N`); beta Linux builds skip RPM for the same reason.
- `Sync installers to OSS` picks the channel from the tag. `Sync updater latest.json to OSS` refuses beta tags.

### OSS storage

- Beta manifest and installers live under `<prefix>/beta/`. Configure an OSS lifecycle rule once per prefix (console → bucket → Data Management → Lifecycle): prefix `desktop/beta/` (and `desktop-team/beta/`), delete objects 30 days after last modified. A beta with no follow-up for 30 days disappears together with its manifest, so the download page simply stops showing it.
- Every stable release keeps the installers and updater packages of the newest **3** stable versions directly under `<prefix>/` and deletes older ones (`scripts/prune-oss-installers.py`). Manifests and `beta/` are never pruned. GitHub Releases keep every version.

### Download page (TeamUps)

Read both manifests:

- `https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop/download.json` — stable, always shown
- `https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop/beta/download.json` — beta; may be missing (404)

Both have the same shape plus `"channel": "stable" | "beta"`. Show the beta block only when the file exists and its `version` is newer than stable (semver, prerelease-aware). Otherwise the beta is already superseded and should be hidden. Label it as a test version that may be unstable; keep stable as the recommended download.

## Why both local and CI

- **Local**: fail in seconds–minutes on the author machine; no empty GitHub release.
- **CI**: last line of defense if someone bypasses the hook.
