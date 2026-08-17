# Desktop (Tauri companion)

**Tauri 2** tray + window that calls the same Rust core as the CLI (`agent-doctor-core`).

Typical loop: **scan → diagnose → confirm repair → Ask re-check**.

![Agent Doctor desktop](../docs/screenshot-desktop.png)

## Features

- System tray: **Show**, **Run doctor**, **Quit** (tooltip: health + workspace + personal/team mode; busy while tray actions run)
- **Agents** — environment health, runtime inventory, Diagnose → Repair → Ask drawer
- **Resources** — Skills / MCP inventory; Browser MCP into Codex / Claude / Hermes / OpenClaw
- **Wiring** — exclusive personal provider vs Evotown team mode (URL templates, verify, apply)
- **Workspace** — list/switch project isolation, remote VPS read-only doctor, Hermes scene presets
- Repair apply / rollback for supported runtimes (Hermes, OpenClaw, DeepSeek Harness, plus Claude/Codex gateway + Browser MCP)
- No separate business logic in the TypeScript UI layer

See [docs/product-boundary.md](../docs/product-boundary.md) for personal vs team modes.

## Develop

```bash
cd desktop
npm install
npm run tauri dev
```

## Build

```bash
cd desktop
npm run tauri build
```

## CLI-only workflow

You can use Agent Doctor without the desktop app:

```bash
cargo run -p agent-doctor -- doctor
```
