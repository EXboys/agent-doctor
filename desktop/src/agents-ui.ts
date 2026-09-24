import { t, type MessageKey } from "./i18n";
import { escapeHtml } from "./format";
import { appState } from "./app-state";
import type {
  HermesSettings,
  RepairPreviewResponse,
  RuntimeDoctorResult,
  RuntimeVersionStatus,
} from "./types";

export const RUNTIME_SHORT: Record<string, string> = {
  openclaw: "OC",
  hermes: "HE",
  "claude-code": "CC",
  codex: "CX",
  "deepseek-harness": "DSH",
};

export const ASK_RUNTIME_IDS = new Set([
  "claude-code",
  "codex",
  "hermes",
  "openclaw",
  "deepseek-harness",
]);

export const BROWSER_MCP_RUNTIME_IDS = new Set([
  "claude-code",
  "codex",
  "hermes",
  "openclaw",
]);

export function isAskRuntimeId(runtimeId: string): boolean {
  return ASK_RUNTIME_IDS.has(runtimeId);
}

export function supportsBrowserMcp(runtimeId: string): boolean {
  return BROWSER_MCP_RUNTIME_IDS.has(runtimeId);
}

export function runtimeClass(id: string): string {
  if (id in RUNTIME_SHORT) {
    return id;
  }
  return "default";
}

export function metaRow(labelKey: MessageKey, value: string, detailTitle?: string): string {
  const compact = value.replace(/\s*\n\s*/g, " · ");
  const title = detailTitle ?? compact;
  return `
    <div class="meta-row">
      <span class="meta-label">${t(labelKey)}</span>
      <p class="meta-value" title="${escapeHtml(title)}">${escapeHtml(compact)}</p>
    </div>
  `;
}

export function renderVersionCompareRow(
  runtime: RuntimeDoctorResult,
  status: RuntimeVersionStatus | undefined,
): string {
  if (!runtime.installed) {
    return "";
  }
  const localRaw = status?.installed || runtime.version;
  if (!localRaw && !status) {
    return metaRow("meta.version", t("meta.versionChecking"));
  }
  if (!localRaw) {
    return "";
  }

  if (!status || status.status === "unknown" || !status.latest) {
    return metaRow("meta.version", t("meta.versionUnknown", { local: localRaw }));
  }

  if (status.status === "up_to_date") {
    return metaRow(
      "meta.version",
      t("meta.versionUpToDate", { local: localRaw }),
    );
  }

  const latest = status.latest;
  const value = t("meta.versionLocalLatest", { local: localRaw, latest });
  const hintParts = [t("meta.versionUpdateHint")];
  if (status.recommended && status.recommended !== latest) {
    hintParts.push(t("meta.versionRecommend", { recommended: status.recommended }));
  }
  const hint = hintParts.join(" ");
  return `
    <div class="meta-row meta-row-version is-update">
      <span class="meta-label">${t("meta.version")}</span>
      <div class="meta-version-block">
        <p class="meta-value" title="${escapeHtml(value)}">${escapeHtml(value)}</p>
        <p class="meta-version-hint">${escapeHtml(hint)}</p>
      </div>
    </div>
  `;
}

export function renderApiKeyRow(settings: HermesSettings): string {
  if (!settings.api_key_env) {
    return metaRow("meta.apiKey", t("meta.apiKeyOptional"));
  }
  if (settings.api_key_configured && settings.api_key_hint) {
    return metaRow(
      "meta.apiKey",
      t("meta.apiKeySet", { hint: settings.api_key_hint }),
    );
  }
  return metaRow(
    "meta.apiKey",
    t("meta.apiKeyMissing"),
    settings.api_key_env || undefined,
  );
}

export function genericRuntimeAdvancedMeta(runtime: RuntimeDoctorResult): string {
  return [
    runtime.profile.key_source ? metaRow("meta.secrets", runtime.profile.key_source) : "",
    runtime.binary_path
      ? metaRow("meta.binary", t("meta.binaryFound"), runtime.binary_path)
      : "",
    runtime.config_paths.length
      ? metaRow(
          "meta.config",
          t("meta.configCount", { count: String(runtime.config_paths.length) }),
          runtime.config_paths.join(" · "),
        )
      : "",
  ]
    .filter(Boolean)
    .join("");
}

export function hermesAdvancedMeta(
  runtime: RuntimeDoctorResult,
  hermesModel: HermesSettings | null,
): string {
  const model = hermesModel;
  const keyNeedsAttention = Boolean(
    model?.api_key_env && !model.api_key_configured,
  );
  return [
    model?.provider ? metaRow("meta.provider", model.provider) : "",
    model?.base_url ? metaRow("meta.gateway", model.base_url) : "",
    model && !keyNeedsAttention ? renderApiKeyRow(model) : "",
    genericRuntimeAdvancedMeta(runtime),
  ]
    .filter(Boolean)
    .join("");
}

export function runtimeAdvancedMeta(
  runtime: RuntimeDoctorResult,
  hermesModel: HermesSettings | null,
): string {
  return runtime.id === "hermes"
    ? hermesAdvancedMeta(runtime, hermesModel)
    : genericRuntimeAdvancedMeta(runtime);
}

export type RuntimeCardActionContext = {
  preview?: RepairPreviewResponse;
  confirmPending: boolean;
  diagnoseOpenForRuntime: boolean;
  dismissed: boolean;
  hasActiveWorkspace: boolean;
  isAskRuntime: boolean;
  supportsBrowserMcp: boolean;
};

export function canOpenSession(runtimeId: string): boolean {
  return isAskRuntimeId(runtimeId);
}

export function runtimeHasProblems(preview?: RepairPreviewResponse): boolean {
  if (!preview) {
    return false;
  }
  return preview.summary.fail > 0 || preview.summary.warn > 0;
}

const ACTION_ICON = {
  diagnose:
    '<svg class="btn-action-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="11" cy="11" r="7" stroke="currentColor" stroke-width="2"/><path d="M20 20l-3.5-3.5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="M8 11h6M11 8v6" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  ask:
    '<svg class="btn-action-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="2" stroke="currentColor" stroke-width="2"/><path d="M8 20h8M12 16v4" stroke="currentColor" stroke-width="2" stroke-linecap="round"/><path d="M7 8h4M7 11h6" stroke="currentColor" stroke-width="2" stroke-linecap="round"/></svg>',
  native:
    '<svg class="btn-action-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2" stroke="currentColor" stroke-width="2"/><path d="M7 9l3 3-3 3M12 15h5" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
  uninstall:
    '<svg class="btn-action-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 7h16M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>',
} as const;

function actionButton(
  kind: "primary" | "secondary",
  action: string,
  icon: string,
  label: string,
  opts?: { title?: string; extraAttrs?: string },
): string {
  const title = opts?.title ? ` title="${escapeHtml(opts.title)}"` : "";
  const extra = opts?.extraAttrs ? ` ${opts.extraAttrs}` : "";
  return `<button type="button" class="btn-${kind} btn-with-icon" data-action="${action}"${title}${extra}>${icon}<span>${escapeHtml(label)}</span></button>`;
}

export function renderStandardRuntimeActions(_runtimeId: string): string {
  return [
    actionButton("primary", "diagnose-runtime", ACTION_ICON.diagnose, t("runtime.diagnose"), {
      title: t("runtime.diagnoseHint"),
    }),
    actionButton("secondary", "ask-session", ACTION_ICON.ask, t("runtime.ask"), {
      title: t("runtime.askHint"),
    }),
    actionButton("secondary", "open-session", ACTION_ICON.native, t("runtime.openTerminal"), {
      title: t("runtime.openTerminalHint"),
      extraAttrs: 'data-open-terminal="1"',
    }),
    actionButton("secondary", "uninstall-runtime", ACTION_ICON.uninstall, t("runtime.uninstall"), {
      title: t("runtime.uninstallHint"),
    }),
  ].join("");
}

export function renderRuntimeCardActions(
  runtime: RuntimeDoctorResult,
  advancedContent = "",
  ctx: RuntimeCardActionContext,
): string {
  if (!runtime.installed) {
    return `<button type="button" class="btn-primary" data-action="install-runtime">${t("runtime.install")}</button>`;
  }

  if (!ctx.isAskRuntime) {
    return canOpenSession(runtime.id)
      ? `<button type="button" class="btn-primary" data-action="open-session">${t("runtime.open")}</button>`
      : "";
  }

  const parts: string[] = [renderStandardRuntimeActions(runtime.id)];
  const canRepair = Boolean(ctx.preview?.can_apply_repair);
  const detailOpenForThis = ctx.diagnoseOpenForRuntime;

  if (!ctx.hasActiveWorkspace) {
    parts.push(
      `<button type="button" class="btn-secondary" data-action="activate-workspace">${t("runtime.activateWorkspace")}</button>`,
    );
  }
  if (canRepair && !detailOpenForThis) {
    parts.push(
      `<button type="button" class="btn-secondary" data-action="apply-repair">${t("repair.oneClick")}</button>`,
    );
  }

  parts.push(`
    <details class="runtime-advanced">
      <summary>${escapeHtml(t("runtime.advanced"))}</summary>
      ${advancedContent ? `<div class="runtime-advanced-meta">${advancedContent}</div>` : ""}
      <div class="runtime-advanced-actions">
        <button type="button" class="btn-ghost" data-action="wire-runtime" title="${escapeHtml(t("runtime.wireRuntimeHint"))}">${t("runtime.wireRuntime")}</button>
        <button type="button" class="btn-ghost" data-action="force-reinstall-runtime" title="${escapeHtml(t("runtime.forceReinstallHint"))}">${t("runtime.forceReinstall")}</button>
      </div>
    </details>
  `);

  return parts.join("");
}

export function renderHermesCard(
  runtime: RuntimeDoctorResult,
  hermesModel: HermesSettings | null,
  actionsHtml: string,
  relatedResourcesHtml: string,
): string {
  const model = hermesModel ?? {
    provider: "",
    model: "",
    base_url: runtime.profile.gateway_url ?? "",
    api_key_env: null,
    api_key_configured: false,
    api_key_hint: null,
  };

  const keyNeedsAttention = Boolean(model.api_key_env && !model.api_key_configured);
  const providerLabel =
    model.provider === "custom"
      ? model.base_url.toLowerCase().includes("deepseek")
        ? `DeepSeek · ${t("meta.openaiCompatible")}`
        : t("meta.openaiCompatible")
      : model.provider;
  const modelSummary = [providerLabel, model.model].filter(Boolean).join(" · ");
  const versionStatus = appState.runtimeVersions.get(runtime.id);
  const summaryMeta = [
    renderVersionCompareRow(runtime, versionStatus),
    modelSummary ? metaRow("meta.model", modelSummary) : "",
    keyNeedsAttention ? renderApiKeyRow(model) : "",
  ]
    .filter(Boolean)
    .join("");
  const badgeClass = keyNeedsAttention ? "warn" : "ok";
  const badgeText = keyNeedsAttention
    ? t("runtime.configAttention")
    : t("runtime.installed");

  return `
    <article class="runtime hermes" data-runtime="hermes">
      <div class="section-label runtime-card-label">
        <h2 class="runtime-tab-title">${escapeHtml(runtime.display_name)}</h2>
        <span class="badge ${badgeClass}">${badgeText}</span>
      </div>
      ${summaryMeta ? `<div class="meta-grid">${summaryMeta}</div>` : ""}
      ${actionsHtml ? `<div class="card-actions">${actionsHtml}</div>` : ""}
      ${relatedResourcesHtml}
      <div class="card-hint repair-hint" data-repair-hint hidden></div>
    </article>
  `;
}

export function renderRuntimeCard(
  runtime: RuntimeDoctorResult,
  hermesModel: HermesSettings | null,
  actionsHtml: string,
  relatedResourcesHtml: string,
): string {
  if (runtime.id === "hermes" && runtime.installed) {
    return renderHermesCard(runtime, hermesModel, actionsHtml, relatedResourcesHtml);
  }

  const state = runtime.installed ? t("runtime.installed") : t("runtime.notInstalled");
  const badgeClass = runtime.installed ? "ok" : "muted";
  const versionStatus = appState.runtimeVersions.get(runtime.id);
  const versionRow = renderVersionCompareRow(runtime, versionStatus);
  const summaryMeta = runtime.installed
    ? [
        versionRow,
        runtime.profile.gateway_url ? metaRow("meta.gateway", runtime.profile.gateway_url) : "",
      ]
        .filter(Boolean)
        .join("")
    : metaRow("meta.status", t("runtime.notDetected"));
  const previewBadge =
    runtime.id === "deepseek-harness"
      ? `<span class="badge preview">${escapeHtml(t("runtime.developerPreview"))}</span>`
      : "";

  return `
    <article class="runtime ${runtimeClass(runtime.id)}" data-runtime="${runtime.id}">
      <div class="section-label runtime-card-label">
        <h2 class="runtime-tab-title">${escapeHtml(runtime.display_name)}</h2>
        <span class="runtime-badges">
          ${previewBadge}
          <span class="badge ${badgeClass}">${state}</span>
        </span>
      </div>
      ${summaryMeta ? `<div class="meta-grid">${summaryMeta}</div>` : ""}
      ${actionsHtml ? `<div class="card-actions">${actionsHtml}</div>` : ""}
      ${relatedResourcesHtml}
      <div class="card-hint repair-hint" data-repair-hint hidden></div>
      ${
        !runtime.installed
          ? `<p class="footnote runtime-install-footnote">${escapeHtml(t("runtime.installHint"))}</p>`
          : ""
      }
    </article>
  `;
}

export function resolveActiveRuntimeId(
  runtimes: RuntimeDoctorResult[],
  activeRuntimeId: string | null,
): string | null {
  if (runtimes.length === 0) {
    return null;
  }
  if (activeRuntimeId && runtimes.some((runtime) => runtime.id === activeRuntimeId)) {
    return activeRuntimeId;
  }
  return runtimes.find((runtime) => runtime.installed)?.id ?? runtimes[0].id;
}

export function runtimeTabDotClass(
  runtime: RuntimeDoctorResult,
  preview?: RepairPreviewResponse,
): string {
  if (!runtime.installed) {
    return "off";
  }
  if (runtimeHasProblems(preview)) {
    return "warn";
  }
  return "ok";
}

export type RepairPreviewLookup =
  | Map<string, RepairPreviewResponse>
  | ((id: string) => RepairPreviewResponse | undefined);

function lookupPreview(
  previews: RepairPreviewLookup,
  runtimeId: string,
): RepairPreviewResponse | undefined {
  return typeof previews === "function" ? previews(runtimeId) : previews.get(runtimeId);
}

export function runtimeListName(runtime: { id: string; display_name: string }): string {
  if (runtime.id === "claude-code") return "Claude";
  if (runtime.id === "codex") return "Codex";
  return runtime.display_name.replace(/\s+(Code|CLI)$/i, "");
}

export function renderRuntimeTabs(
  runtimes: RuntimeDoctorResult[],
  selectedId: string,
  previews: RepairPreviewLookup,
): string {
  return runtimes
    .map((runtime) => {
      const active = runtime.id === selectedId;
      const preview = lookupPreview(previews, runtime.id);
      const shortName = runtimeListName(runtime);
      const stateLabel = !runtime.installed
        ? t("runtime.notInstalled")
        : runtimeHasProblems(preview)
          ? t("runtime.configAttention")
          : t("runtime.installed");
      const versionStatus = appState.runtimeVersions.get(runtime.id);
      const versionMeta =
        versionStatus?.status === "update_available" && versionStatus.latest
          ? t("meta.versionLocalLatest", {
              local: versionStatus.installed || runtime.version || "—",
              latest: versionStatus.latest,
            })
          : runtime.version;
      const tabMeta = [stateLabel, versionMeta].filter(Boolean).join(" · ");
      const displayMeta =
        runtime.id === "deepseek-harness"
          ? [t("runtime.experimentalShort"), tabMeta].filter(Boolean).join(" · ")
          : tabMeta;
      return `
        <button
          type="button"
          class="runtime-tab ${runtimeClass(runtime.id)} ${active ? "is-active" : ""}"
          role="tab"
          aria-selected="${active}"
          data-runtime-tab="${runtime.id}"
        >
          <span class="runtime-tab-dot ${runtimeTabDotClass(runtime, preview)}" aria-hidden="true"></span>
          <span class="runtime-tab-label">${escapeHtml(shortName)}</span>
          <span class="runtime-tab-meta">${escapeHtml(displayMeta)}</span>
        </button>
      `;
    })
    .join("");
}
