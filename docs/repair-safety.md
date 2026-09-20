# Repair Safety and Compliance Model

Agent Doctor treats repair as a controlled enterprise workflow, not as a free-form agent with shell access.

## Principles

- **Local first**: probes, backups, repair plans, and audit reports are local by default.
- **Redacted by default**: secrets and sensitive logs must be redacted before an AI model sees diagnostic data.
- **Typed actions only**: repair execution is limited to known action kinds such as backup, config patch, install/update, symlink cleanup, verify, and manual review.
- **Confirmation for writes**: file changes, reinstalls, and destructive operations require explicit confirmation.
- **Rollback metadata**: every real repair run must produce a backup snapshot and audit report.
- **Policy gates**: enterprise policy can block raw log upload, raw config upload, destructive actions, or non-approved gateways.

## Diagnostic Data Classes

| Class | Examples | Default AI visibility |
|-------|----------|-----------------------|
| `public` | runtime id, installed flag, version | visible |
| `local_path` | config path, binary path, backup root | redacted to `$HOME` form |
| `config_shape` | field names, schema errors, parse errors | visible |
| `secret` | API keys, bearer tokens, OAuth tokens | hidden |
| `sensitive_log` | logs that may include prompts, tokens, file names | inline secrets redacted |

## Repair Flow

1. Collect deterministic probe facts:
   - binary existence and `--version`
   - PATH/default binary and duplicate install candidates
   - config existence, parse status, and runtime-specific schema warnings
   - environment variable conflicts from process and common shell files
   - gateway/base_url TCP reachability
   - obvious MCP/Skills path references and broken links
   - Hermes provider/API key env requirements and `.env` permissions
  - Claude Code base URL / API key / settings permissions
  - Codex provider schema (`wire_api`, `openai_base_url`, env_key) and placeholder auth.json
2. Create a backup snapshot before modifying files.
3. Redact the diagnostic bundle.
4. Let AI summarize and rank likely causes from redacted facts.
5. Convert the plan into typed repair actions.
6. Ask for confirmation before writes.
7. Execute the approved actions.
8. Verify runtime health and write an audit report.

## Enterprise Controls

Teams can require:

- Company gateway only for AI diagnosis.
- No raw secrets, logs, or config values outside the local machine.
- Mandatory backup snapshot before repair.
- Mandatory diff preview before config writes.
- High-risk actions disabled unless policy explicitly allows them.
- Local audit reports for IT support or control-plane ingestion.

`agent-doctor repair <runtime>` runs read-only probes and prints the safe repair preview.

`agent-doctor repair <runtime> --apply` runs the execute skeleton:

1. `probe_runtime` (before)
2. build repair plan from redacted facts
3. backup runtime config files under `~/.config/agent-doctor/backups/`
4. apply typed actions (rule playbooks still expanding; unimplemented actions are skipped)
5. `probe_runtime` (after)
6. write `AuditReport` with verification summary and rollback hint

Runtime-specific rule playbooks extend step 4. **Hermes v1 (shipped):**

- tighten `~/.hermes/.env` permissions to `600` when too open
- deduplicate repeated API key env entries (keep last non-empty value)
- fill missing/empty `model.*` fields from the active Agent Doctor profile preset
- when the provider API key is missing: append a `VAR=` placeholder to `~/.hermes/.env` (or create the file) and write a local guide under `~/.config/agent-doctor/guides/hermes-api-key-<VAR>.md`
- when `hermes` is not on PATH: run the official Hermes installer script (same approach as [CC Switch](https://github.com/farion1231/cc-switch); requires network and user confirmation via `--apply`)
- when `openclaw` is not on PATH: run the official OpenClaw installer (`openclaw.ai/install.sh` with `--no-onboard --no-prompt`; requires network and `--apply`)

**OpenClaw v1 (shipped):**

- install binary when missing
- create `openclaw.json` when config missing (uses company gateway from profile.env when set)
- apply `gateway.url` from `profile.env` when gateway not configured
- migrate legacy `agents.defaults.timeout` → `timeoutSeconds`
- fix `env.vars` / `env.shellEnv` stored as JSON strings → objects
- reset invalid `tools.profile` to `coding`
- tighten `~/.openclaw/.env` permissions to `600`
- dedupe API key lines in `.env`; scaffold `env.vars` placeholders + local guide (no secret fill)

**Claude Code / Codex v1 (shipped):**

- install via `npm install -g` when binary missing
- create / rewrite config from active Personal/Team mode (gateway, provider slot, model)
- Browser MCP wire/repair
- Claude: empty `ANTHROPIC_API_KEY` scaffold + local guide; tighten `settings.json` to `600` when it holds a key
- Codex: set `wire_api=responses` (+ align `openai_base_url`); clear placeholder/empty `auth.json`; API key env guide (no secret fill)
- heal bare/`default` model and provider schema drift via mode rewire

**Never shipped / not planned for v1:** auto-filling or uploading API keys; AI choosing arbitrary shell commands.

**Rollback:** backups live under `~/.config/agent-doctor/backups/<runtime>-<timestamp>/`. Restore with:

```bash
agent-doctor repair hermes --rollback
agent-doctor repair hermes --rollback --backup hermes-2026-05-28T12-00-00
```

The desktop app shows suggested fixes after diagnosis, runs apply (backup → playbook → re-probe → audit), can open the API key guide, and can roll back from the latest backup.

## Rule-based install + AI install

`agent-doctor install <runtime>` works for every registered runtime:

| Layer | Behavior |
|-------|----------|
| Rule install | When lifecycle hooks exist (Hermes, OpenClaw, Claude Code, Codex): official script / `npm install -g` → re-probe; logs under `~/.config/agent-doctor/logs/` |
| No rules | Skip rule phase → **AI install** via allowlisted bash |
| Rule failed | **AI repair loop** automatically (read logs, retry allowlisted install, config fixes) |
| After success | Optional `--plan ai` / `--repair` for remaining config issues |

| Flag | Behavior |
|-------|----------|
| `--explain` | AI plain-language summary (probe + optional install log tail) |
| `--plan ai` / `--repair` | After successful install, run AI or deterministic repair for other issues |

Bash is **not** arbitrary: each runtime registers allowlisted install commands only.

## Bounded repair loop (generic orchestration)

`agent-doctor repair <runtime> --loop` runs a runtime-agnostic loop (up to 5 rounds) through the unified runtime registry:

1. `probe_runtime`
2. `suggest_runtime_repairs` (per-runtime rules, registered in `RUNTIME_REGISTRY`)
3. build **masked** repair context (secrets as `{{SECRET:n}}` vault tokens — never sent to an LLM payload)
4. plan fixes (`--plan deterministic` default; `--plan ai` reserved for a future LLM planner that only returns typed `action_ids`)
5. with `--apply --loop`: `apply_runtime_playbook` (typed actions + backup already taken)
6. re-probe and stop when there is no progress or no auto-fixable work remains

Preview without writes:

```bash
agent-doctor repair hermes --loop
agent-doctor install openclaw              # rule-based install (official script)
agent-doctor install openclaw --explain      # install + AI failure/success explanation
agent-doctor install openclaw --plan ai      # install + AI repair loop for remaining issues
agent-doctor repair openclaw --explain     # AI diagnosis from probe (no writes)
agent-doctor doctor --explain                # AI diagnosis per runtime with issues
```

Execute:

```bash
agent-doctor repair hermes --apply --loop
agent-doctor repair hermes --apply --loop --plan ai   # placeholder: same as deterministic today
```

New runtimes only need registry entries and playbooks; they do not need a separate loop implementation. Secret **restore** after any future LLM `.env` proposals happens locally via `merge_env_with_vault` (reject model-invented keys).

## Agent tools (`read` / `edit` / `bash`)

With `--plan ai`, the repair loop runs an **agent tool layer** before playbook execution:

| Tool | To LLM | Local execute |
|------|--------|----------------|
| `list_dir` | file/dir names + sizes only | scoped to adapter config roots |
| `grep_files` | `path:line:masked_text` hits | literal search under allowed paths |
| `read_file` | numbered + masked body (`{{SECRET:n}}`) | reads disk, masks into vault |
| `search_replace` | `old_string` / `new_string` with tokens | unmask → unique match → replace → diff preview |
| `write_file` | full masked content | unmask → write (small/new files only) |
| `patch_config` | `key_path` + scalar value | YAML/JSON/TOML dot-path set |
| `bash` | masked command | unmask → allowlist → run |

Secrets never leave the machine in plaintext. The session `SecretVault` is `#[serde(skip)]` on planner context.

Configure the planner LLM via environment:

- `AGENT_DOCTOR_LLM_API_KEY` or `OPENAI_API_KEY`
- `AGENT_DOCTOR_LLM_API_URL` (default OpenAI-compatible `/v1/chat/completions`)
- `AGENT_DOCTOR_LLM_MODEL` (default `gpt-4o-mini`)

Without an API key, `--plan ai` falls back to the deterministic planner.

Bash is **not** a free shell: only Hermes/OpenClaw install/update, `hermes|openclaw --version`, and `chmod` on `.env` paths are allowed. Edits are limited to runtime config paths from the adapter registry.
