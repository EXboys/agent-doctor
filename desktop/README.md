# Desktop (Tauri companion)

**Tauri 2** tray + window that calls the same Rust core as the CLI (`agent-doctor-core`).

Two windows share the Rust core: **chat** for daily use, and the tray **home** window for scan and repair.

![Agent Doctor desktop](../docs/screenshot-desktop.png)

## Features

- **Chat** — projects, the conversation, the current folder, knowledge base, scheduled tasks, skills and tools. A signed-in computer accepts a Feishu sentence. Voice is in settings.
- **Home** — health, then Agent, resources, model service, and projects. Scan, diagnose, confirm a repair.
- System tray: **Show**, **Run doctor**, **Check for updates**, **Quit** (tooltip: health + workspace + edition; busy while tray actions run)
- Personal builds show your model service. Team builds show the company gateway. Two installers, two update channels.
- Auto-update via Tauri Updater (China CDN first, GitHub fallback) — see [docs/desktop-auto-update.md](../docs/desktop-auto-update.md)
- Repair apply / rollback for supported runtimes (Hermes, OpenClaw, DeepSeek Harness, plus Claude/Codex gateway + Browser MCP)
- No separate business logic in the TypeScript UI layer

See [docs/product-boundary.md](../docs/product-boundary.md) for personal vs team **editions**.

## Develop

```bash
cd desktop
npm install
npm run tauri:dev:personal   # TeamUps, the default package
npm run tauri:dev:team       # enterprise package
```

## Build

```bash
cd desktop
npm run tauri:build:personal   # TeamUps — com.agentdoctor.app, updater → desktop/
npm run tauri:build:team       # enterprise — com.agentdoctor.team, updater → desktop-team/
```

See [docs/desktop-auto-update.md](../docs/desktop-auto-update.md) for updater channels.

## CLI-only workflow

You can use Agent Doctor without the desktop app:

```bash
cargo run -p agent-doctor -- doctor
```
