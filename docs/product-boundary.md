# Product boundary: Personal vs Team editions

Agent Doctor is one codebase with a shared ops core. **Personal** and **team** ship as **separate install packages** (editions), not as an in-app mode switch.

Set `AGENT_DOCTOR_EDITION=personal|team` at build time (default: `personal`).

## Beginners are the default user (locked)

People using the desktop app are beginners. They do not read a terminal, and they do not know MCP, file paths, protocols, or CLI commands.

Design and copy are for that person.

1. Say what is wrong in one sentence, using words they already have: project, tool, cannot connect, needs a key.
2. Button labels name the change they will see, for example “Put in this project”. Do not label a control “Fix” when it cannot clear the problem.
3. After the click, show the result in the same place they clicked. If the warning is still there, say why in one plain sentence and what to tap next.
4. Commands, paths, and raw English errors are never the only explanation. Those can sit behind a details control.

## Shared core

| Capability | Meaning |
|------------|---------|
| **doctor** | Discover runtimes, probe config/gateway drift, explain failures |
| **repair** | Backup → typed playbook fixes → re-probe → audit (ops repair, not a “butler”) |
| **workspace** | Project isolation so memories/MCP/skills do not cross-contaminate |

## Personal edition (TeamUps)

| | |
|--|--|
| **Package** | `AGENT_DOCTOR_EDITION=personal` (default) |
| **Entry** | TeamUps / consumer desktop + CLI |
| **Wiring** | Personal provider only (no Evotown / team tab) |
| **Value** | When agents break: diagnose, repair, keep projects isolated |
| **Do not** | Become a personal proxy marketplace / usage dashboard product |

Personal provider means: wire an endpoint + key + model, verify, write runtime configs, and repair schema/gateway wiring. **URL/model templates** (DeepSeek, OpenRouter, …) are fine as fill-in helpers.

## Team edition (enterprise)

| | |
|--|--|
| **Package** | `AGENT_DOCTOR_EDITION=team` |
| **Entry** | Enterprise-customized desktop + CLI |
| **Wiring** | Evotown / company gateway only (no personal provider UI) |
| **Value** | Compliance, baseline, sync, dispatch, audit |
| **Increments** | `setup` / `sync` / `policy` / `connect` + audit/compliance export |
| **Do not** | Depend on personal proxy ecosystems for team compliance |

Evotown is the control plane (accounts, SkillHub, policy, gateway, dispatch, audit ingest). Agent Doctor remains the on-laptop executor and repair tool. See [enterprise.md](enterprise.md).

## Wiring pipeline (within an edition)

LLM wiring still goes through **`apply_mode_switch`**: resolve credentials → write overlay → project each runtime → effectors → LLM probe. The edition gate refuses the other path (personal package cannot apply team; team package cannot apply personal).

Team OpenClaw/Codex/Hermes default model is a routable id (not bare `default`). Additive slots:

| Runtime | Slots | Pointer |
|---------|-------|---------|
| OpenClaw | `models.providers.evotown` + `personal` | `agents.defaults.model.primary` |
| Codex | `model_providers.company` + `personal` | `model_provider` |
| Hermes | `~/.hermes/agent-doctor-slots.yaml` | `model.base_url` / `model.default` |

Doctor probes: `mode.overlay_mismatch`, `runtime.env_stale` (OpenClaw keys), `runtime.model_unroutable`. See issue [#46](https://github.com/EXboys/agent-doctor/issues/46).

## Profile state (do not mix)

| File | Role |
|------|------|
| `{config_dir}/agent-doctor/settings.db` + local secrets file | **Authoritative** Doctor settings and secrets (`secrets.json` on Unix, DPAPI `secrets.dpapi` on Windows) |
| `~/.config/agent-doctor/profile.env` | Legacy active runtime overlay projection |
| `~/.config/agent-doctor/company-profile.env` | Legacy durable team baseline projection |
| `~/.config/agent-doctor/providers.json` | Legacy personal providers metadata (keys in local secrets vault) |
| `~/.config/evotown/evotown.agent.env` | Legacy Evotown connection projection |

Skills source: user/team override when set; otherwise default **TeamUps**. Mount never requires a remote source.

Workspace **company baseline** drift checks against `company-profile.env` only.

## Narrative rules

1. Default story: laptop ops — doctor / repair / workspace.
2. Evotown ships only in the **team** edition, not as a toggle inside the TeamUps package.
3. UI copy for personal setup stays in wiring/repair language (“endpoint”, “verify”, “apply to runtimes”), not marketplace language (“pick a provider plan”).
4. Hermes scene `profile` presets (local model scenes) are workspace/dev convenience, not a personal proxy catalog — keep them scoped to Hermes scene switching.

## Desktop build

```bash
cd desktop
npm run tauri:build:personal   # TeamUps — com.agentdoctor.app → desktop/ CDN
npm run tauri:build:team       # enterprise — com.agentdoctor.team → desktop-team/ CDN
```

Bundle ID and updater endpoints are edition-specific so the two packages never share updates. See [desktop-auto-update.md](desktop-auto-update.md).
