# Bundled CLI resources

`agent-doctor-cli` is copied here by `desktop/scripts/prebuild-cli.sh` (also `npm run bundle:cli`).

Tauri packs this directory into the desktop app so Browser MCP configure works without a separate PATH install.

- macOS: `Agent Doctor.app/Contents/Resources/agent-doctor-cli`
- Windows / Linux: beside the app binary under `resources/`

Do not commit the binary; it is gitignored.
