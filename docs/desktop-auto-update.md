# Desktop auto-update (China CDN first)

Agent Doctor desktop uses [Tauri Updater](https://v2.tauri.app/plugin/updater/).

## Endpoints (client)

Configured in `desktop/src-tauri/tauri.conf.json`:

1. `https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop/latest.json` — **primary** (mainland China)
2. `https://github.com/EXboys/agent-doctor/releases/latest/download/latest.github.json` — fallback (GitHub asset URLs)

Updater only moves to the next endpoint on non-2XX (or transport failure). Keep the CDN healthy.

Manual download button / failed-update dialog opens the same CDN prefix.

## One-time setup

### 1. Signing key (required)

A keypair was generated for this repo locally (gitignored):

- private: `desktop/src-tauri/.updater-private.key`
- public: already embedded as `plugins.updater.pubkey` in `tauri.conf.json`

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

Current bucket: `agent-doctor` / `oss-cn-shenzhen`. Set the bucket (or at least the `desktop/` prefix) to **public-read**, then add:

| Secret | Value |
| --- | --- |
| `UPDATE_CDN_BASE_URL` | `https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop` |
| `OSS_ACCESS_KEY_ID` | Aliyun AK |
| `OSS_ACCESS_KEY_SECRET` | Aliyun SK |
| `OSS_BUCKET` | `agent-doctor` |
| `OSS_ENDPOINT` | `oss-cn-shenzhen.aliyuncs.com` |
| `OSS_PREFIX` | `desktop` |

If OSS secrets are missing, Release still uploads updater JSON/packages to GitHub; mainland auto-update will not work until CDN sync is configured.

## Release flow

On `v*` tags, Release CI:

1. Builds desktop with `createUpdaterArtifacts` + signing env
2. Uploads installers / `.sig` / updater archives to GitHub Releases
3. Runs `scripts/build-updater-manifest.py` → `latest.json` (CDN URLs) + packages
4. Uploads `latest.json` to GitHub Release
5. Syncs `latest.json` + packages to OSS when secrets are present

## Local verify

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat desktop/src-tauri/.updater-private.key)"
cd desktop && npm run tauri -- build
# serve target/release/bundle/... + a handmade latest.json over HTTPS and point endpoints at it
```

`tauri dev` does not exercise updater installs; use a signed release build.
