/**
 * Headless smoke for extracted desktop UI modules (no Tauri).
 * Run: cd desktop && npx tsx --import ./scripts/smoke-dom-shim.ts scripts/smoke-ui-modules.ts
 */
import {
  isAskRuntimeId,
  renderRuntimeCard,
  renderRuntimeCardActions,
  renderRuntimeTabs,
  supportsBrowserMcp,
} from "../src/agents-ui";
import {
  preferredRepairFilter,
  renderDiagnosePendingHtml,
  renderRepairPreview,
} from "../src/repair-ui";
import {
  AskResourcesController,
  dedupeMcpsByName,
  mcpChipLabel,
  mergeMentionsForSend,
  promptRequestsBrowserMcp,
} from "../src/ask-resources";
import type { McpInventoryItem, RepairPreviewResponse, RuntimeDoctorResult } from "../src/types";

function assert(cond: unknown, msg: string): asserts cond {
  if (!cond) throw new Error(msg);
}

const runtime: RuntimeDoctorResult = {
  id: "hermes",
  display_name: "Hermes",
  installed: true,
  version: "1.0.0",
  binary_path: "/tmp/hermes",
  config_paths: ["/tmp/.hermes/config.yaml"],
  profile: { gateway_url: "https://example.com/v1", key_source: "env" },
};

const preview: RepairPreviewResponse = {
  runtime_id: "hermes",
  display_name: "Hermes",
  summary: { pass: 2, warn: 1, fail: 1, not_applicable: 0, not_checked: 0 },
  checks: [
    {
      title: "Gateway",
      status: "fail",
      message: "unreachable",
      details: ["timeout"],
    },
    {
      title: "Config",
      status: "warn",
      message: "missing key",
      details: [],
    },
    {
      title: "Binary",
      status: "pass",
      message: "ok",
      details: [],
    },
  ],
  plan_summary: "fix gateway",
  suggested_repairs: [
    {
      id: "fix-hermes-config-from-profile",
      title: "Restore config",
      description: "from profile",
      auto_fixable: true,
    },
  ],
  can_apply_repair: true,
  backup_ids: ["b1"],
  last_execute: null,
};

// 1) Agents refresh / card render
assert(isAskRuntimeId("hermes"), "hermes is ask runtime");
assert(supportsBrowserMcp("hermes"), "hermes supports browser mcp");
const actions = renderRuntimeCardActions(runtime, "<div>adv</div>", {
  preview,
  confirmPending: false,
  diagnoseOpenForRuntime: false,
  dismissed: false,
  hasActiveWorkspace: true,
  isAskRuntime: true,
  supportsBrowserMcp: true,
});
assert(actions.includes('data-action="apply-repair"'), "one-click repair button");
assert(actions.includes('data-action="ask-session"'), "ask button");

const card = renderRuntimeCard(
  runtime,
  null,
  actions,
  '<div data-related-resources></div>',
);
assert(card.includes('data-runtime="hermes"'), "hermes card");
assert(card.includes("card-actions"), "card actions mounted");

const tabs = renderRuntimeTabs([runtime], "hermes", new Map([["hermes", preview]]));
assert(tabs.length > 0, "tabs render");

// 2) Diagnose panel
assert(preferredRepairFilter(preview) === "fail", "prefer fail filter");
const pending = renderDiagnosePendingHtml("hermes", "Hermes", "Scanning…", "diagnose");
assert(pending.includes("is-pending"), "pending diagnose html");
assert(pending.includes("Scanning"), "pending message");

const panel = renderRepairPreview(preview, "fail", {
  confirmPending: true,
  isAskRuntime: true,
  supportsBrowserMcp: true,
});
assert(panel.includes("repair-panel"), "repair panel");
assert(
  panel.includes("repair-confirm") || panel.includes('data-action="confirm-repair"'),
  "confirm repair UI",
);

const panelAll = renderRepairPreview(preview, "all", {
  confirmPending: false,
  isAskRuntime: true,
  supportsBrowserMcp: true,
});
assert(panelAll.includes("repair-check"), "checks list");
assert(
  panelAll.includes('data-action="preview-repair"') ||
    panelAll.includes('data-action="apply-repair"') ||
    panelAll.includes("repair-funnel"),
  "funnel/actions present",
);

// 3) Ask resources panel helpers
const shellOpen = { open: false };
const controller = new AskResourcesController(
  {
    shellEl: {
      classList: {
        toggle(_token: string, force?: boolean) {
          if (typeof force === "boolean") shellOpen.open = force;
          else shellOpen.open = !shellOpen.open;
        },
        contains: () => shellOpen.open,
      },
    } as unknown as HTMLElement,
    resourcesPanelEl: { setAttribute() {} } as unknown as HTMLElement,
    resourcesToggleEl: {
      classList: { toggle() {} },
      setAttribute() {},
    } as unknown as HTMLButtonElement,
    resourcesLabelEl: { textContent: "", title: "" } as unknown as HTMLElement,
    skillsListEl: { replaceChildren() {}, appendChild() {} } as unknown as HTMLElement,
    mcpListEl: { replaceChildren() {}, appendChild() {} } as unknown as HTMLElement,
    skillsEmptyEl: { hidden: false } as unknown as HTMLElement,
    mcpEmptyEl: { hidden: false } as unknown as HTMLElement,
    mentionsEl: {
      replaceChildren() {},
      appendChild() {},
      hidden: true,
    } as unknown as HTMLElement,
  },
  () => "hermes",
  () => "/tmp/proj",
  (cwd) => cwd,
);
controller.mountedSkills = [
  {
    skill_id: "demo",
    name: "Demo",
    version: "1",
    description: null,
    installed_path: "/tmp",
    agents: [{ runtime: "hermes", scope: "user", path: "/tmp", mounted: true }],
    call_count: null,
    success_count: null,
    success_rate: null,
    first_success_rate: null,
    download_count: null,
    metrics_source: "local",
  },
];
controller.enabledMcps = [
  {
    name: "browser",
    scope: "user",
    config_path: "/tmp",
    command: "agent-doctor",
    args: ["mcp", "browser"],
    healthy: true,
    issue: null,
    is_browser: true,
    runtime_hint: "hermes",
  },
];
controller.upsertMention({ kind: "skill", id: "demo", label: "Demo" });
assert(controller.selectedMentions.length === 1, "mention selected");
assert(promptRequestsBrowserMcp("open browser and screenshot"), "browser prompt detect");
assert(promptRequestsBrowserMcp("navigate to https://example.com"), "navigate prompt detect");
const duplicateBrowsers: McpInventoryItem[] = [
  {
    name: "browser",
    scope: "openclaw-workspace",
    config_path: "/ws/.mcp.json",
    command: "agent-doctor",
    args: ["mcp", "browser"],
    healthy: true,
    issue: null,
    is_browser: true,
    runtime_hint: "openclaw",
  },
  {
    name: "browser",
    scope: "openclaw-global",
    config_path: "/home/.openclaw/openclaw.json",
    command: "agent-doctor",
    args: ["mcp", "browser"],
    healthy: true,
    issue: null,
    is_browser: true,
    runtime_hint: "openclaw",
  },
  {
    name: "browser",
    scope: "claude-settings-ignored",
    config_path: "/home/.claude/settings.json",
    command: "agent-doctor",
    args: ["mcp", "browser"],
    healthy: false,
    issue: "legacy",
    is_browser: true,
    runtime_hint: "shared",
  },
];
const deduped = dedupeMcpsByName(duplicateBrowsers);
assert(deduped.length === 1, "dedupe three browser bindings to one chip");
assert(deduped[0].scope === "openclaw-workspace", "prefer workspace binding");
assert(mcpChipLabel(deduped[0]) === "browser", "chip label not browser · browser");
const merged = mergeMentionsForSend(
  "use @skill:demo please",
  controller.selectedMentions,
  controller.mountedSkills,
  controller.enabledMcps,
);
assert(merged.some((m) => m.kind === "skill" && m.id === "demo"), "merge mentions");

// 4) Ask resources panel toggle
controller.setResourcesOpen(true);
assert(shellOpen.open, "resources panel open");
controller.setResourcesOpen(false);
assert(!shellOpen.open, "resources panel closed");
controller.toggleResourcesPanel();
assert(shellOpen.open, "resources panel toggled open");
assert(controller.selectedMentions.length === 1, "mentions survive toggle");
controller.renderResourceChips();
controller.renderMentions();
controller.updateResourcesSummary();

console.log("smoke-ui-modules: OK");
console.log(
  JSON.stringify(
    {
      step1_agentsCard: actions.includes('data-action="apply-repair"'),
      step2_diagnosePending: pending.includes("is-pending"),
      step3_repairConfirm:
        panel.includes("confirm-repair") || panel.includes("repair-confirm"),
      step4_askResourcesToggle: shellOpen.open,
      step4_askMentions: controller.selectedMentions.length === 1,
    },
    null,
    2,
  ),
);
