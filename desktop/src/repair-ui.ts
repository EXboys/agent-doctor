import { t, type MessageKey } from "./i18n";
import { escapeHtml } from "./format";
import type {
  RelatedTag,
  RepairPreviewResponse,
  RepairStatusFilter,
} from "./types";

function renderRepairSummaryChip(
  filter: RepairStatusFilter,
  count: number,
  className: string,
  label: string,
  activeFilter: RepairStatusFilter,
): string {
  const isActive = activeFilter === filter;
  const disabled = count === 0;
  return `
    <button
      type="button"
      class="repair-chip ${className}${isActive ? " is-active" : ""}"
      data-repair-filter="${filter}"
      aria-pressed="${isActive}"
      ${disabled ? "disabled" : ""}
    >
      ${count} ${label}
    </button>
  `;
}

export function repairCheckStatusLabel(
  status: RepairPreviewResponse["checks"][number]["status"],
): string {
  switch (status) {
    case "pass":
      return t("repair.pass");
    case "warn":
      return t("repair.warn");
    case "fail":
      return t("repair.fail");
    case "n/a":
      return t("repair.notApplicable");
    case "not checked":
      return t("repair.notChecked");
    default:
      return status;
  }
}

export function repairStatusClass(
  status: RepairPreviewResponse["checks"][number]["status"],
): string {
  if (status === "pass") {
    return "pass";
  }
  if (status === "warn") {
    return "warn";
  }
  if (status === "fail") {
    return "fail";
  }
  return "muted";
}

const REPAIR_FIX_LABEL_KEYS: Record<string, string> = {
  "backup-runtime-configs": "repair.fix.backup",
  "fix-hermes-env-permissions": "repair.fix.envPermissions",
  "fix-hermes-api-key-duplicates": "repair.fix.apiKeyDedupe",
  "fix-hermes-api-key-scaffold": "repair.fix.apiKeyScaffold",
  "fix-hermes-config-from-profile": "repair.fix.configFromProfile",
  "fix-claude-code-gateway-from-mode": "repair.fix.claudeGateway",
  "fix-codex-gateway-from-mode": "repair.fix.codexGateway",
  "fix-claude-code-browser-mcp": "repair.fix.claudeBrowserMcp",
  "fix-codex-browser-mcp": "repair.fix.codexBrowserMcp",
  "fix-hermes-browser-mcp": "repair.fix.hermesBrowserMcp",
  "fix-openclaw-browser-mcp": "repair.fix.openclawBrowserMcp",
};

export function repairFixLabel(actionId: string): string {
  const key = REPAIR_FIX_LABEL_KEYS[actionId];
  return key ? t(key as MessageKey) : actionId;
}

export function formatVerificationSummary(summary: string): string {
  const match = summary.match(/^before:\s*(.+?);\s*after:\s*(.+)$/);
  if (!match) {
    return summary;
  }
  return `${match[1]} → ${match[2]}`;
}

export function renderRepairExecuteResult(
  execute: NonNullable<RepairPreviewResponse["last_execute"]>,
  supportsBrowserMcp: boolean,
  offerVerify = false,
): string {
  const playbookExecuted = execute.executed.filter((id) => id.startsWith("fix-"));
  const hasBackup = execute.executed.includes("backup-runtime-configs");

  const executedLines = execute.executed.map((id) => repairFixLabel(id));

  const outcome =
    playbookExecuted.length === 0 && execute.skipped.length === 0
      ? `<p class="repair-execute-ok">${escapeHtml(t("repair.nothingToFix"))}</p>`
      : "";

  const executedBlock =
    executedLines.length > 0
      ? `<p><strong>${escapeHtml(t("repair.executed"))}:</strong> ${escapeHtml(executedLines.join("、"))}</p>`
      : "";

  const skippedBlock =
    execute.skipped.length > 0
      ? `<p><strong>${escapeHtml(t("repair.skipped"))}:</strong> ${escapeHtml(
          execute.skipped.map((item) => `${repairFixLabel(item.id)} (${item.reason})`).join("；"),
        )}</p>`
      : "";

  const verify = formatVerificationSummary(execute.verification_summary);

  const canVerifyBrowserMcp = supportsBrowserMcp;
  const smoke = canVerifyBrowserMcp && execute.browser_smoke
    ? `<p class="repair-browser-smoke ${execute.browser_smoke.ok ? "ok" : "fail"}"><strong>${escapeHtml(
        execute.browser_smoke.ok ? t("repair.browserSmokeOk") : t("repair.browserSmokeFail"),
      )}</strong> ${escapeHtml(execute.browser_smoke.detail)}</p>`
    : "";

  const guideBlock = execute.guide_path
    ? `<p class="repair-guide"><button type="button" class="btn-link repair-guide-btn" data-action="open-repair-guide" data-guide-path="${encodeURIComponent(execute.guide_path)}">${escapeHtml(t("repair.openGuide"))}</button></p>`
    : "";

  return `
    <div class="repair-execute-result">
      ${
        hasBackup
          ? `<p class="repair-execute-backup">${escapeHtml(t("repair.applyResult", { backup: execute.backup_root }))}</p>`
          : ""
      }
      ${outcome}
      ${executedBlock}
      ${skippedBlock}
      ${guideBlock}
      <p class="repair-verify"><strong>${escapeHtml(t("repair.verifyTitle"))}:</strong> ${escapeHtml(verify)}</p>
      ${smoke}
      ${
        offerVerify && canVerifyBrowserMcp
          ? `<p class="repair-funnel-hint">${escapeHtml(t("repair.funnelAskVerifyHint"))}</p>
      <button type="button" class="btn-primary" data-action="ask-verify">${escapeHtml(t("repair.funnelAskVerifyCta"))}</button>`
          : offerVerify
            ? `<button type="button" class="btn-primary" data-action="ask-session">${escapeHtml(t("runtime.ask"))}</button>`
            : ""
      }
    </div>
  `;
}

export function renderRepairPreview(
  report: RepairPreviewResponse,
  activeFilter: RepairStatusFilter = "all",
  opts?: {
    confirmPending?: boolean;
    isAskRuntime?: boolean;
    supportsBrowserMcp?: boolean;
  },
): string {
  const summary = report.summary;
  const visibleChecks =
    activeFilter === "all"
      ? report.checks
      : report.checks.filter((check) => check.status === activeFilter);

  const checks = visibleChecks
    .map((check) => {
      const statusClass = repairStatusClass(check.status);
      const details = check.details.length
        ? `<span class="repair-check-detail">${escapeHtml(check.details[0])}${check.details.length > 1 ? ` +${check.details.length - 1}` : ""}</span>`
        : "";
      const legacyAgents = check.message.includes("agents.list is a legacy key");
      const title = legacyAgents ? t("repair.openclawLegacyAgentsTitle") : check.title;
      const message = legacyAgents ? t("repair.openclawLegacyAgentsDesc") : check.message;
      return `
        <li class="repair-check is-${statusClass}">
          <span class="repair-check-status ${statusClass}">${escapeHtml(repairCheckStatusLabel(check.status))}</span>
          <span class="repair-check-body">
            <strong>${escapeHtml(title)}</strong>
            <span>${escapeHtml(message)}</span>
            ${details}
          </span>
        </li>
      `;
    })
    .join("");

  const summaryChips = [
    { filter: "all" as const, count: report.checks.length, className: "all", label: t("repair.all") },
    { filter: "pass" as const, count: summary.pass, className: "pass", label: t("repair.pass") },
    { filter: "warn" as const, count: summary.warn, className: "warn", label: t("repair.warn") },
    { filter: "fail" as const, count: summary.fail, className: "fail", label: t("repair.fail") },
    {
      filter: "not checked" as const,
      count: summary.not_checked,
      className: "muted",
      label: t("repair.notChecked"),
    },
    {
      filter: "n/a" as const,
      count: summary.not_applicable,
      className: "muted",
      label: t("repair.notApplicable"),
    },
  ]
    .filter((chip) => chip.filter === "all" || chip.count > 0)
    .map((chip) =>
      renderRepairSummaryChip(chip.filter, chip.count, chip.className, chip.label, activeFilter),
    )
    .join("");

  const emptyList =
    visibleChecks.length === 0
      ? `<li class="repair-check repair-check-empty">${escapeHtml(t("repair.noMatches"))}</li>`
      : "";

  const suggested = report.suggested_repairs.length
    ? `
      <div class="repair-suggested">
        <p class="repair-suggested-title">${escapeHtml(t("repair.suggestedTitle"))}</p>
        <ul class="repair-suggested-list">
          ${report.suggested_repairs
            .map((item) => {
              const plain =
                item.id === "review-claude-global-mcp"
                  ? {
                      badge: t("repair.claudeGlobalMcpBadge"),
                      title: t("repair.claudeGlobalMcpTitle"),
                      description: t("repair.claudeGlobalMcpDesc"),
                    }
                  : item.id === "fix-openclaw-legacy-agents-list"
                    ? {
                        badge: t("repair.autoFixable"),
                        title: t("repair.openclawLegacyAgentsTitle"),
                        description: t("repair.openclawLegacyAgentsDesc"),
                      }
                    : null;
              const manualAction =
                item.id === "configure-openclaw-api-key" ||
                item.id === "configure-deepseek-harness-credentials"
                  ? `<button type="button" class="btn-ghost repair-suggested-action" data-action="go-wiring">${escapeHtml(t("repair.goWiring"))}</button>`
                  : item.id === "review-claude-global-mcp"
                    ? `<button type="button" class="btn-primary btn-compact repair-suggested-action" data-action="migrate-claude-mcp">${escapeHtml(t("repair.migrateClaudeMcp"))}</button>`
                    : "";
              return `
                <li class="repair-suggested-item">
                  <span class="repair-suggested-badge ${item.auto_fixable ? "ok" : "muted"}">${
                    plain
                      ? escapeHtml(plain.badge)
                      : item.auto_fixable
                        ? t("repair.autoFixable")
                        : t("repair.manualOnly")
                  }</span>
                  <span class="repair-suggested-body">
                    <strong>${escapeHtml(plain?.title ?? item.title)}</strong>
                    <span>${escapeHtml(plain?.description ?? item.description)}</span>
                  </span>
                  ${manualAction}
                </li>
              `;
            })
            .join("")}
        </ul>
      </div>
    `
    : "";

  const rollbackButton =
    report.backup_ids.length > 0
      ? `<button type="button" class="btn-ghost repair-rollback-btn" data-action="rollback-repair">${t("repair.rollback")}</button>`
      : "";

  const healthy = summary.fail === 0 && summary.warn === 0;
  const supportsBrowserMcp = Boolean(opts?.supportsBrowserMcp);
  const executeResult = report.last_execute
    ? renderRepairExecuteResult(report.last_execute, supportsBrowserMcp, healthy)
    : "";

  const isAskRuntime = Boolean(opts?.isAskRuntime);
  const canVerifyBrowserMcp = supportsBrowserMcp;
  // Warn/fail plus an auto-fix still needs a repair action, even after a previous execute.
  const funnelNeedsRepair = isAskRuntime && report.can_apply_repair && !healthy;
  const showRepairConfirm = funnelNeedsRepair && Boolean(opts?.confirmPending);
  const canMigrateClaudeMcp = report.suggested_repairs.some(
    (item) => item.id === "review-claude-global-mcp",
  );
  const funnel = isAskRuntime
    ? `<div class="repair-funnel">
        <div class="repair-funnel-bar">
          <ol class="repair-funnel-steps">
            <li class="repair-funnel-step done">${escapeHtml(t("repair.funnelStepDiagnose"))}</li>
            <li class="repair-funnel-step ${report.last_execute && healthy ? "done" : (report.can_apply_repair || canMigrateClaudeMcp) && !healthy ? "active" : healthy ? "done" : ""}">${escapeHtml(t("repair.funnelStepRepair"))}</li>
            <li class="repair-funnel-step ${healthy ? "active" : ""}">${escapeHtml(t("repair.funnelStepAsk"))}</li>
          </ol>
          ${
            funnelNeedsRepair && !showRepairConfirm
              ? `<button type="button" class="btn-primary" data-action="preview-repair">${escapeHtml(t("repair.oneClick"))}</button>`
              : ""
          }
        </div>
        ${
          report.runtime_id === "openclaw"
            ? `<p class="repair-funnel-hint" title="${escapeHtml(t("repair.openclawMcpNote"))}">${escapeHtml(t("repair.openclawMcpNote"))}</p>`
            : ""
        }
      </div>`
    : "";
  const autoFixItems = report.suggested_repairs.filter((item) => item.auto_fixable);
  const repairConfirm = showRepairConfirm
    ? `
      <div class="repair-confirm-card">
        <div class="repair-confirm-head">
          <span class="repair-confirm-icon" aria-hidden="true">↻</span>
          <span>
            <strong>${escapeHtml(t("repair.previewTitle"))}</strong>
            <small>${escapeHtml(t("repair.previewCount", { count: String(autoFixItems.length) }))}</small>
          </span>
        </div>
        <ul class="repair-confirm-list">
          ${autoFixItems
            .map(
              (item) =>
                `<li><strong>${escapeHtml(item.title)}</strong><span>${escapeHtml(item.description)}</span></li>`,
            )
            .join("")}
        </ul>
        <div class="repair-confirm-safety">
          <span>${escapeHtml(t("repair.previewBackup"))}</span>
          <span>${escapeHtml(t("repair.previewNoSecrets"))}</span>
          <span>${escapeHtml(t("repair.previewRecheck"))}</span>
        </div>
        <div class="repair-confirm-actions">
          <button type="button" class="btn-secondary" data-action="cancel-repair-preview">${escapeHtml(t("repair.cancel"))}</button>
          <button type="button" class="btn-primary" data-action="confirm-repair">${escapeHtml(t("repair.confirmFix"))}</button>
        </div>
      </div>
    `
    : "";
  const applyButton =
    report.can_apply_repair && !funnelNeedsRepair
      ? `<button type="button" class="btn-primary repair-apply-btn" data-action="apply-repair">${t("repair.applyFixes")}</button>`
      : "";
  const headBits = [
    `${summary.pass} ${t("repair.pass")}`,
    summary.warn ? `${summary.warn} ${t("repair.warn")}` : "",
    summary.fail ? `${summary.fail} ${t("repair.fail")}` : "",
    summary.not_checked ? `${summary.not_checked} ${t("repair.notChecked")}` : "",
  ].filter(Boolean);

  return `
    <div class="repair-panel" data-runtime="${escapeHtml(report.runtime_id)}">
      <div class="repair-panel-head">
        <strong>${escapeHtml(report.display_name)}</strong>
        <span>${escapeHtml(headBits.join(" · "))}</span>
        <button type="button" class="repair-panel-close" data-action="close-diagnose-detail">${escapeHtml(t("repair.closeDetail"))}</button>
      </div>
      ${funnel}
      ${repairConfirm}
      ${showRepairConfirm ? "" : suggested}
      ${
        showRepairConfirm
          ? ""
          : `<div class="repair-summary" role="tablist" aria-label="${escapeHtml(t("repair.filterLabel"))}">
              ${summaryChips}
            </div>
            <ul class="repair-checks">${checks}${emptyList}</ul>`
      }
      ${
        !showRepairConfirm && (applyButton || rollbackButton)
          ? `<div class="repair-panel-actions">${applyButton}${rollbackButton}</div>`
          : ""
      }
      ${showRepairConfirm ? "" : executeResult}
      ${
        canVerifyBrowserMcp && !showRepairConfirm
          ? `<div class="repair-smoke-row">
              <button type="button" class="btn-ghost" data-action="browser-smoke">${escapeHtml(t("repair.runBrowserSmoke"))}</button>
              <span class="repair-smoke-slot" data-browser-smoke-slot></span>
            </div>`
          : ""
      }
    </div>
  `;
}

export function preferredRepairFilter(report: RepairPreviewResponse): RepairStatusFilter {
  if (report.summary.fail > 0) {
    return "fail";
  }
  if (report.summary.warn > 0) {
    return "warn";
  }
  return "all";
}

export function extractRelatedTags(report: RepairPreviewResponse | undefined): RelatedTag[] {
  if (!report) {
    return [];
  }
  const tags: RelatedTag[] = [];
  const seen = new Set<string>();
  for (const check of report.checks) {
    const blob = `${check.title} ${check.message}`;
    if (!/mcp|skill/i.test(blob)) {
      continue;
    }
    const kind: RelatedTag["kind"] = /skill/i.test(blob) && !/mcp/i.test(check.title)
      ? "skill"
      : "mcp";
    const nameMatch =
      check.message.match(/[`'"]([a-z0-9._/-]+)[`'"]/i) ||
      check.title.match(/(?:mcp|skill)[._-]?([a-z0-9._/-]+)/i);
    const name = (nameMatch?.[1] || check.title).replace(/^(mcp|skill)[._-]?/i, "") || check.title;
    const key = `${kind}:${name}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    tags.push({
      kind,
      name,
      broken: check.status === "fail" || check.status === "warn",
    });
  }
  return tags;
}

export function renderRelatedResourcesHtml(report: RepairPreviewResponse | undefined): string {
  const tags = extractRelatedTags(report);
  const broken = tags.filter((tag) => tag.broken);
  if (tags.length === 0) {
    return `<div class="mounted is-empty" data-related-resources hidden></div>`;
  }
  if (broken.length === 0) {
    return `
      <div class="mounted is-quiet" data-related-resources>
        <span class="tone-soft">${escapeHtml(t("agents.relatedOk", { n: String(tags.length) }))}</span>
      </div>
    `;
  }
  const tagsHtml = broken
    .map((tag) => {
      const label = tag.kind === "skill" ? "Skill" : "MCP";
      return `<span class="mini-tag ${tag.kind} broken">${label} · ${escapeHtml(tag.name)} · ${t("agents.relatedMissing")}</span>`;
    })
    .join("");
  return `
    <div class="mounted" data-related-resources>
      <h3>${escapeHtml(t("agents.relatedTitle"))}</h3>
      <div class="mini-tags">${tagsHtml}</div>
    </div>
  `;
}

export function renderDiagnosePendingHtml(
  runtimeId: string,
  displayName: string,
  message: string,
  step: "diagnose" | "repair" = "diagnose",
): string {
  const diagnoseClass = step === "diagnose" ? "active" : "done";
  const repairClass = step === "repair" ? "active" : "";
  return `
    <div class="repair-panel is-pending" data-runtime="${escapeHtml(runtimeId)}">
      <div class="repair-panel-head">
        <strong>${escapeHtml(displayName)}</strong>
        <span>${escapeHtml(message)}</span>
        <button type="button" class="repair-panel-close" data-action="close-diagnose-detail">${escapeHtml(t("repair.closeDetail"))}</button>
      </div>
      <div class="repair-funnel">
        <div class="repair-funnel-bar">
          <ol class="repair-funnel-steps">
            <li class="repair-funnel-step ${diagnoseClass}">${escapeHtml(t("repair.funnelStepDiagnose"))}</li>
            <li class="repair-funnel-step ${repairClass}">${escapeHtml(t("repair.funnelStepRepair"))}</li>
            <li class="repair-funnel-step">${escapeHtml(t("repair.funnelStepAsk"))}</li>
          </ol>
        </div>
      </div>
      <div class="repair-pending">
        <span class="spinner" aria-hidden="true"></span>
        <span>${escapeHtml(message)}</span>
      </div>
      ${step === "repair" ? `<p class="repair-funnel-hint">${escapeHtml(t("repair.applyingHint"))}</p>` : ""}
    </div>
  `;
}
