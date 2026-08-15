# Install Agent Doctor

## Prerequisites

- [GitHub CLI](https://cli.github.com/) (`gh`) for downloading release assets
- Or download files manually from [Releases](https://github.com/EXboys/agent-doctor/releases)

## CLI

Pick the archive that matches your OS and CPU:

| Asset pattern | Platform |
|---------------|----------|
| `agent-doctor-*-macos-arm64.tar.gz` | macOS Apple Silicon |
| `agent-doctor-*-macos-x86_64.tar.gz` | macOS Intel |
| `agent-doctor-*-linux-x86_64.tar.gz` | Linux x86_64 |
| `agent-doctor-*-windows-x86_64.zip` | Windows x86_64 |

### macOS (Apple Silicon)

```bash
gh release download --repo EXboys/agent-doctor --pattern 'agent-doctor-*-macos-arm64.tar.gz'
tar -xzf agent-doctor-*-macos-arm64.tar.gz
chmod +x agent-doctor
sudo mv agent-doctor /usr/local/bin/
agent-doctor doctor
```

### macOS (Intel)

```bash
gh release download --repo EXboys/agent-doctor --pattern 'agent-doctor-*-macos-x86_64.tar.gz'
tar -xzf agent-doctor-*-macos-x86_64.tar.gz
chmod +x agent-doctor
sudo mv agent-doctor /usr/local/bin/
agent-doctor doctor
```

### Linux

```bash
gh release download --repo EXboys/agent-doctor --pattern 'agent-doctor-*-linux-x86_64.tar.gz'
tar -xzf agent-doctor-*-linux-x86_64.tar.gz
chmod +x agent-doctor
sudo mv agent-doctor /usr/local/bin/
agent-doctor doctor
```

### Windows (PowerShell)

```powershell
gh release download --repo EXboys/agent-doctor --pattern "agent-doctor-*-windows-x86_64.zip"
Expand-Archive agent-doctor-*-windows-x86_64.zip -DestinationPath .
Move-Item .\agent-doctor.exe "$env:LOCALAPPDATA\Programs\agent-doctor\"
```

## Desktop (menubar app)

After a release is published, download the desktop bundle for your platform from the same GitHub release:

- **macOS**: `.dmg`
- **Windows**: `.msi` or `.exe` setup
- **Linux**: `.AppImage` or `.deb` (when built by Tauri)

```bash
# List desktop assets for the latest release
gh release view --repo EXboys/agent-doctor --json assets --jq '.assets[].name'
gh release download --repo EXboys/agent-doctor --pattern '*.dmg'
```

## Build from source

```bash
# CLI
cargo install --path cli --locked

# Desktop: bundle CLI into Tauri resources (required for Browser MCP on clean installs)
cd desktop
npm install
npm run bundle:cli
npm run tauri dev
```

`npm run tauri -- build` already runs `scripts/prebuild-cli.sh` via `beforeBuildCommand`, so release `.app` / installers ship `agent-doctor-cli` under `Contents/Resources/` (macOS) or beside the exe (Windows/Linux).

### Browser CDP smoke (Chrome / Edge)

```bash
cargo build --release -p agent-doctor
./target/release/agent-doctor mcp smoke --browser chrome --url https://example.com/
./target/release/agent-doctor mcp smoke --browser edge --url https://example.com/
```

CI runs the same smoke matrix on every PR (see `.github/workflows/ci.yml`).

## Create a release (maintainers)

Push a version tag to trigger `.github/workflows/release.yml`:

```bash
git tag v0.1.0
git push origin v0.1.0
```

This builds CLI archives for all platforms and attaches Tauri desktop installers to the GitHub release.
