# Coverage Discipline

One codebase manages many resource types. This document governs how wide that coverage may grow without diluting a solo-maintained project. Layer ownership is defined in [control-plane.md](control-plane.md).

## Two kinds of coverage

**Managed coverage (thin adapter).** A new resource is admitted through the shared machinery — probes, diagnosis rules, lifecycle verbs, typed repair. The marginal cost is an adapter plus rules. This is the product; it can grow.

**Deep coverage (deep cell).** Building product-grade depth inside a managed layer — for example a distributed-inference orchestrator, a model marketplace, or a provider-switching UI. Deep cells burn the solo heat budget and break neutrality: Agent Doctor starts favoring its own component inside a layer it arbitrates. Deep cells are rationed.

## Depth policy per layer

| Layer | The one deep cell allowed | Everything else |
|-------|---------------------------|-----------------|
| L4 life entry | none — never own | consume events only |
| L3 agent runtimes | **repair**: diagnose + typed fix + rollback | install/config stay thin; provider switching is left to dedicated tools |
| L2 compute | **out-of-box default brain**: cluster MVP per [zh-CN/exo-cluster.md](zh-CN/exo-cluster.md) | no orchestrator ambitions, no model marketplace |
| L1 OS/hardware | none | call OS APIs only |

## Rules

**R1 — One deep cell in flight.** At most one deep cell is under active development at any time. Everything else is thin adapters or frozen. A new deep cell trades places with the current one; it never stacks. (Current: L2 cluster MVP. The resident reconciliation daemon from [control-plane.md](control-plane.md) is the designated successor, not a parallel track.)

**R2 — The three-question gate.** Before starting any new resource or deep cell, all three answers must be yes:

1. Does it reuse the shared probe / rule / repair machinery?
2. Does it directly serve the out-of-box story or the personal showcase?
3. Is its maintenance surface a thin adapter, not a new product line?

Any no → the item goes to the thin list or stays frozen.

**R3 — Depth budget until revenue or a second maintainer.** The adapter list may grow; depth may not. 5 runtime adapters already mean every upstream config-format change is our work. Before adding the 6th, ask whether it brings new users or just completeness.

**R4 — Adapters stay thin.** An adapter implements the lifecycle verbs for exactly one resource against the shared registry. It must not bring its own UI, its own persistence, or a protocol that bypasses shared machinery (no raw `Command::new` for long-running processes — see the raw spawn guard test in `crates/agent-doctor-core/tests/`).

**R5 — Quarterly audit.** Every deep cell must show flywheel feedback: repairs feeding the diagnosis rule catalog, revenue, or measurable adoption. A deep cell that cannot demonstrate it gets demoted to a thin adapter or frozen. Layer coverage that is wide but shallow per cell is not lethal; several deep cells at once is.

## Why this document exists

Integration layers historically die by spreading before the wedge earns its place — not by competitors. The wedge here is repair. The test: when a user's agent breaks, Agent Doctor should be the first thing they think of. Until that is true, every new cell is borrowed against the wedge.
