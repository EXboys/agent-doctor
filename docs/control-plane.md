# Control Plane Charter

Agent Doctor is the **control plane for local AI resources**. It manages the install, configuration, health, repair, and recovery of resources it does not execute: agent runtimes (OpenClaw, Hermes, DeepSeek Harness, Claude Code, Codex), compute backends (exo, Ollama, llama.cpp, cloud APIs), and tool surfaces (MCP, skills).

This document fixes the architectural identity. Product scope lives in [product-boundary.md](product-boundary.md). Write safety lives in [repair-safety.md](repair-safety.md). Coverage growth lives in [coverage-discipline.md](coverage-discipline.md).

## Layer ownership

| Layer | Contents | Agent Doctor's role |
|-------|----------|---------------------|
| L4 | Life entry: calendar, messaging, home control | **Never own.** Consume events only (e.g. one Feishu sentence routed to this computer) |
| L3 | Agent runtimes | **Manage**: install, configure, diagnose, repair, recover |
| L2 | Compute and models | **Manage**: personal out-of-box brain (see [zh-CN/exo-cluster.md](zh-CN/exo-cluster.md)) |
| L1 | OS and hardware | **Consume OS APIs only.** Never ship an OS, drivers, or kernel services |

Every managed layer is arbitrated horizontally. Agent Doctor does not compete vertically inside any layer it manages.

## Control plane vs data plane (locked)

Agent Doctor does **not** execute the workloads it manages.

- Agent runtimes run their own processes and their own agent loops.
- Inference engines own their schedulers, model placement internals, and device access.
- Agent Doctor owns the state **around** execution: discovery, configuration, policy, health, backup and rollback, audit.

A feature that requires owning data-plane execution — a first-party agent runtime, a first-party inference scheduler competing with managed engines — is out of scope by definition. Propose it in a separate project, not here.

## Organs (internal architecture map)

The operating-system analogy is an internal map of responsibilities, not a marketing claim.

| OS organ | Agent Doctor equivalent |
|----------|-------------------------|
| Driver layer | Resource adapters (runtime + compute backend registry) |
| Device manager | Probes paired with the diagnosis rule catalog ([install-diagnosis-rules.md](install-diagnosis-rules.md)) |
| Service manager | Lifecycle: pinned on-demand download, daemon start/stop, orphan reclaim |
| Security model | Typed actions, backup before write, rollback, audit reports ([repair-safety.md](repair-safety.md)) |
| Scheduler | Scheduled tasks |
| Storage | `settings.db`, knowledge base, secrets file |
| IPC | Tauri commands, cross-window broadcasts |

**Do not** call the product an "AI OS" in public copy. It invites two failure modes: fighting platform vendors on their marketing turf, and expectation lock-in — "since it is an OS, why not also own X" — which is the main scope-creep vector. Public wording stays: installable, repairable, keeps working.

## North star: declared state + reconciliation loop

Today the control plane is imperative: `doctor` and `repair` run when a human asks.

The organ that completes the identity is the **reconciliation loop** (the resident watch daemon):

1. Desired state is declared — explicitly by the user or by shipped defaults: which runtimes should be healthy, which compute cluster should be formed, which model should be placed.
2. The daemon continuously compares desired vs actual state.
3. Drift converges through the **same typed-action machinery** as manual repair: backup → typed fix → re-probe → audit. No silent writes, no free-form shell, ever.
4. Drift it cannot converge surfaces as one plain sentence plus a one-tap fix, per the beginners rule in [product-boundary.md](product-boundary.md).

Until the loop ships, imperative commands are admission points into the same machinery — not a separate path. When the loop ships, manual flows remain the fallback, not a duplicate implementation.

## Neutrality invariant

The control plane's value depends on being neutral between vendors. Every deep cell (see [coverage-discipline.md](coverage-discipline.md)) inside a managed layer must stay replaceable by third-party equivalents and must not receive privileged APIs that peer resources cannot use. A bundled compute backend, for example, must not get exclusive placement endpoints that a user-installed backend lacks.
