# Desktop (Tauri menubar)

**Tauri 2** menubar companion that calls the same Rust core as the CLI (`agent-doctor-core`).

## Features (MVP)

- System tray with **Show**, **Run doctor**, **Quit** (tooltip: health + workspace + personal/team mode; busy while tray actions run)
- Diagnose / personal provider (with URL templates) / Evotown tabs
- Workspace picker: list and switch registered project workspaces
- Hermes scene preset switching and API key status
- Repair apply / rollback for supported runtimes
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
