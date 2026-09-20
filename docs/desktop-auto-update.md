# Desktop auto-update (China CDN first)

Agent Doctor desktop uses [Tauri Updater](https://v2.tauri.app/plugin/updater/).

Personal (TeamUps) and team (enterprise) packages use **different bundle IDs and updater channels**, so they never steal each other's updates.

## Editions

| Edition | Bundle ID | Product name | CDN prefix | GitHub updater JSON |
| --- | --- | --- | --- | --- |
| **personal** (default) | `com.agentdoctor.app` | Agent Doctor | `desktop/` | `latest.json` / `latest.github.json` |
| **team** | `com.agentdoctor.team` | Agent Doctor Team | `desktop-team/` | `latest.team.json` / `latest.team.github.json` |

Configs: `desktop/src-tauri/tauri.personal.conf.json`, `tauri.team.conf.json` (merged via `scripts/tauri-with-edition.sh`).

## Endpoints (client)

### Personal

1. `https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop/latest.json` — **primary** (mainland China)
2. `https://github.com/EXboys/agent-doctor/releases/latest/download/latest.github.json` — fallback

### Team

1. `https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop-team/latest.json`
2. `https://github.com/EXboys/agent-doctor/releases/latest/download/latest.team.github.json`

Updater only moves to the next endpoint on non-2XX (or transport failure). Keep the CDN healthy.

Manual download button / failed-update dialog opens the matching CDN prefix for the running edition.

## One-time setup

### 1. Signing key (required)

A keypair was generated for this repo locally (gitignored):

- private: `desktop/src-tauri/.updater-private.key`
- public: already embedded as `plugins.updater.pubkey` in `tauri.conf.json` (shared by both editions)

Add GitHub Actions secrets:

| Secret | Value |
| --- | --- |
| `TAURI_SIGNING_PRIVATE_KEY` | **full contents** of `.updater-private.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | empty if the key has no password |

If you rotate keys, regenerate with:

```bash
cd desktop
npm run tauri -- signer generate -w src-tauri/.updater-private.key
```

Then replace `plugins.updater.pubkey` with the new `.pub` contents. **Users on the old pubkey cannot verify new updates** — treat rotation as a breaking change.

### 2. China CDN (Aliyun OSS)

Current bucket: `agent-doctor` / `oss-cn-shenzhen`. Set **both** prefixes public-read:

| Prefix | Edition |
| --- | --- |
| `desktop/` | personal |
| `desktop-team/` | team |

| Secret | Value |
| --- | --- |
| `UPDATE_CDN_BASE_URL` | `https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop` (personal; team uses `desktop-team` automatically) |
| `OSS_ACCESS_KEY_ID` | Aliyun AK |
| `OSS_ACCESS_KEY_SECRET` | Aliyun SK |
| `OSS_BUCKET` | `agent-doctor` |
| `OSS_ENDPOINT` | `oss-cn-shenzhen.aliyuncs.com` |

If OSS secrets are missing, Release still uploads updater JSON/packages to GitHub; mainland auto-update will not work until CDN sync is configured.

## Release flow

On `v*` tags, Release CI:

1. Builds desktop per edition matrix row with `tauri-with-edition.sh` + signing env
2. Uploads installers / `.sig` / updater archives to GitHub Releases
3. Runs `scripts/build-updater-manifest.py --edition …` → edition-specific `latest*.json` + packages
4. Uploads those JSON files to GitHub Release
5. Syncs `latest.json` + packages to the edition OSS prefix (`desktop/` or `desktop-team/`)

Tag releases currently build **personal** only. To also ship team, add matching `edition: team` rows in `.github/workflows/release.yml` (desktop matrix + publish matrix).

## Local verify

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat desktop/src-tauri/.updater-private.key)"
cd desktop
npm run tauri:build:personal
# or: npm run tauri:build:team
```

`tauri dev` does not exercise updater installs; use a signed release build.
