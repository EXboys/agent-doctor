# Runtime adapters

Each runtime is registered once in `crates/agent-doctor-core/src/runtime/registry.rs` (`RUNTIME_REGISTRY`). A registry entry wires:

| Capability | Registry field |
|------------|----------------|
| Adapter (`discover`, config paths, model apply) | `create_adapter` |
| Probe (binary name, config format, env keywords) | `probe` |
| Deep probes (runtime-specific checks) | `deep_probe` |
| Config schema validation | `schema_probe` |
| Repair suggestions + playbook | `suggest_repairs`, `apply_playbook` |
| Install / update | `run_lifecycle` |

Adapter implementations live under `crates/agent-doctor-core/src/adapters/`. Use `runtime::all_adapters()` / `runtime::adapter_by_id()` — not a separate adapter list.

## Adding a runtime

1. Implement `RuntimeAdapter` in `adapters/<name>.rs`
2. Add one `RuntimeDescriptor` row to `RUNTIME_REGISTRY` (order = UI / doctor listing order)
3. Implement config schema checks in `probe/runtimes/<name>.rs` and wire `schema_probe`
4. Optional: deep probe fn, repair playbook, lifecycle hooks

## Adapter trait methods

| Method | Description |
|--------|-------------|
| `discover()` | Is runtime installed? Version? Binary path? |
| `config_paths()` | Default config file locations |
| `read_profile()` | Current gateway URL / key source (redacted) |
| `apply_model()` | Merge model preset into runtime config |

## Implemented adapters

| Adapter | Priority | Implementation |
|---------|----------|----------------|
| `openclaw` | P0 | `crates/agent-doctor-core/src/adapters/openclaw.rs` |
| `hermes` | P0 | `crates/agent-doctor-core/src/adapters/hermes.rs` |
| `deepseek-harness` | P0 | `crates/agent-doctor-core/src/adapters/deepseek_harness.rs` |
| `claude-code` | P0 | `crates/agent-doctor-core/src/adapters/claude_code.rs` |
| `codex` | P1 | `crates/agent-doctor-core/src/adapters/codex.rs` |
| `skilllite` | P2 | — |
