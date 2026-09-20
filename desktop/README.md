# Desktop (Tauri companion)

**Tauri 2** tray + window that calls the same Rust core as the CLI (`agent-doctor-core`).

Typical loop: **scan → diagnose → confirm repair → Ask re-check**.

![Agent Doctor desktop](../docs/screenshot-desktop.png)

## Features

- System tray: **Show**, **Run doctor**, **Check for updates**, **Quit** (tooltip: health + workspace + personal/team mode; busy while tray actions run)
- **Agents** — environment health, runtime inventory, Diagnose → Repair → Ask drawer
- **Resources** — Skills / MCP inventory; Browser MCP into Codex / Claude / Hermes / OpenClaw
- **Wiring** — locked by build edition: personal provider (TeamUps) **or** Evotown (enterprise), not both
- **Workspace** — list/switch project isolation, remote VPS read-only doctor, Hermes scene presets
- Auto-update via Tauri Updater (China CDN first, GitHub fallback) — see [docs/desktop-auto-update.md](../docs/desktop-auto-update.md)
- Repair apply / rollback for supported runtimes (Hermes, OpenClaw, DeepSeek Harness, plus Claude/Codex gateway + Browser MCP)
- No separate business logic in the TypeScript UI layer

See [docs/product-boundary.md](../docs/product-boundary.md) for personal vs team **editions**.

## Develop

```bash
cd desktop
npm install
# Personal edition (default / TeamUps)
npm run tauri:dev:personal
# Team edition (enterprise)
npm run tauri:dev:team
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
