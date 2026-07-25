# Product boundary: Personal (C) vs Team (B)

Agent Doctor is one local client with a shared ops core. Personal and team are **modes**, not separate products.

## Shared core

| Capability | Meaning |
|------------|---------|
| **doctor** | Discover runtimes, probe config/gateway drift, explain failures |
| **repair** | Backup → typed playbook fixes → re-probe → audit (ops repair, not a “butler”) |
| **workspace** | Project isolation so memories/MCP/skills do not cross-contaminate |

## Personal (C)

| | |
|--|--|
| **Entry** | CLI + light desktop (“fix it on this machine”) |
| **Value** | When agents break: diagnose, repair, keep projects isolated |
| **Increments** | Zero-config readiness, personal provider wiring, one-click repair — still **ops** semantics |
| **Do not** | Become a personal proxy marketplace / usage dashboard product |

Personal provider means: wire an endpoint + key + model, verify, write runtime configs, and repair schema/gateway wiring. **URL/model templates** (DeepSeek, OpenRouter, …) are fine as fill-in helpers — that is ops convenience, not competing on proxy ecosystems.

## Team (B)

| | |
|--|--|
| **Entry** | Same client + Evotown / company profile |
| **Value** | Compliance, baseline, sync, dispatch, audit |
| **Increments** | `setup` / `sync` / `policy` / `connect` + audit/compliance export |
| **Do not** | Depend on personal proxy ecosystems for team compliance |

Evotown is the control plane (accounts, SkillHub, policy, gateway, dispatch, audit ingest). Agent Doctor remains the on-laptop executor and repair tool. See [enterprise.md](enterprise.md).

## Profile state (do not mix)

| File | Role |
|------|------|
| `~/.config/agent-doctor/profile.env` | **Active** runtime overlay (personal *or* company) |
| `~/.config/agent-doctor/company-profile.env` | **Durable team baseline** (written by company `setup`; never overwritten by personal activate) |
| `~/.config/agent-doctor/personal-providers.json` | Named personal providers store |
| `~/.config/evotown/evotown.agent.env` | Evotown connection (URL / `evk_` / engine id) |

Workspace **company baseline** drift checks against `company-profile.env` only. Activating a personal provider must not make team baseline compare against a personal URL.

## Narrative rules

1. Default story: laptop ops — doctor / repair / workspace.
2. Evotown is an optional team increment, not the only identity.
3. UI copy for personal setup stays in wiring/repair language (“endpoint”, “verify”, “apply to runtimes”), not marketplace language (“pick a provider plan”).
4. Hermes scene `profile` presets (local model scenes) are workspace/dev convenience, not a personal proxy catalog — keep them scoped to Hermes scene switching.
