# Install & Diagnosis Rule Library

A rule is the unit that turns "it does not work" into "this check failed, run this fix".
Every rule has the same three parts:

| Part | Where it lives | What it decides |
|------|----------------|-----------------|
| **Check** | `crates/agent-doctor-core/src/probe/` | reads the computer and emits `ProbeStatus::Pass \| Warn \| Fail` under a stable id |
| **Rule** | `crates/agent-doctor-repair/src/repair/playbooks/` | matches one check id + status to a fix |
| **Fix** | the rule's `fix:` field | runs a typed lifecycle action, or hands the user a manual step |

The point of the library is coverage, not cleverness: when a new "installed but
unusable" failure shows up, it should become one more row here plus one check and
one rule — not another ad-hoc branch in the installer.

## Why a separate library

An npm-installed runtime can be *present and broken at once*. The binary exists,
`--version` exits 0, the config parses, and every existing check is green — while
the product does nothing. Those are exactly the failures worth encoding, because
no single component owns them: the package is fine, Node is fine, and only the
pair is not.

## Rules

### `fix-deepseek-harness-node-version` — Node older than the package needs

- **Applies to**: `deepseek-harness` (any npm-installed runtime can opt in)
- **Check**: `deepseek-harness.node.engine` — `Fail` when the `node` on PATH is
  older than the floor in the installed package's `engines.node`.
- **Facts**: `deepseek-harness.node.required`, `deepseek-harness.node.actual`
- **Symptom**: `dsh --version` prints nothing and exits 0; `--profile acp` exits
  in milliseconds; chat shows "no reply" while the whole diagnose page is green.
- **Cause**: dsh's `lib/bin.js` calls `runCli()` only under `if (import.meta.main)`,
  and `import.meta.main` does not exist before Node 22.18 — the CLI is a no-op
  program rather than a crashing one. Later code paths need
  `zlib.createZstdCompress` (22.15+), and undici wants 22.19+.
- **Fix**: `lifecycle::ensure_node_at_least(minimum)` — installs a satisfying Node
  into `AgentDoctor/runtime/nodejs` and puts it first on PATH. The machine's own
  Node install is never touched, so a working toolchain elsewhere is not broken.
- **Floor source**: the installed package's `engines.node`, resolved by
  `probe::node_engine::minimum_from_range`; `DEEPSEEK_HARNESS_NODE_MIN` is only the
  fallback when the manifest cannot be read.

### `configure-deepseek-harness-credentials` — no API key

- **Check**: `deepseek-harness.api_key.configured` — `Warn` when neither the
  process env, dsh's `.env`, nor Agent Doctor's wiring provides a key.
- **Fix**: manual. Secrets are never auto-filled.

### `fix-deepseek-harness-install` / `-version` — missing or stale package

- **Checks**: `binary.exists`, `deepseek-harness.pinned_version`
- **Fix**: `run_deepseek_harness_lifecycle(Install | Update)`.

## Adding a rule

1. **Find the invariant, not the incident.** "Node must satisfy `engines.node`" is
   an invariant; "this user had 22.12" is not. Prefer reading the requirement from
   the artifact itself over hard-coding a number per runtime.
2. **Emit facts, then a check.** Put the numbers a fix (or an LLM planner) would
   need into `DiagnosticFact`s, marked `SensitivityLevel::Public` when they are
   versions. Checks carry the human sentence; facts carry the machine inputs.
3. **Stay quiet when the input is missing.** If the package is absent or declares
   no floor, emit nothing — the missing-package check already covers it, and a
   second complaint is noise the planner has to filter.
4. **Make the fix reversible or out-of-the-way.** Prefer writing into Agent
   Doctor's own runtime directory over mutating a system-wide install.
5. **Add the row here.** A rule that is not in this table does not exist as far as
   the next person debugging a "green but broken" install is concerned.

## Checklist for a "green but broken" report

When every check passes and the product still does nothing, the invariant is
usually *between* components. Check, in order:

1. **The interpreter** — does the CLI run on the Node/Ruby/Python actually on PATH?
   (`node --version` from the same environment the app inherits.)
2. **The entry point** — does the CLI's own guard condition hold on this version?
   (`if (import.meta.main)`, `if __name__ == "__main__"`, feature-detect APIs.)
3. **Silent exit codes** — exit 0 with empty output is a failure mode, not a pass.
   A version probe should treat "no version parsed" as unknown, never as success.
4. **Inherited environment** — a desktop app inherits PATH from Explorer at launch;
   a shell fix does not reach it until the app restarts.
