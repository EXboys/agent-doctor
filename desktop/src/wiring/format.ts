import { t } from "../i18n";
import type { ModeSwitchReport } from "../types";

/** Always write Browser MCP when applying provider — no separate toggle. */
export function wantsBrowserMcp(): boolean {
  return true;
}

export function formatBrowserMcpHint(report: ModeSwitchReport): string | null {
  const results = report.browser_mcp?.results;
  if (!results?.length) {
    return null;
  }
  const ok = results.filter((item) => item.ok).length;
  const fail = results.find((item) => !item.ok);
  if (fail) {
    return t("mode.browserMcpFail", { detail: fail.message });
  }
  return t("mode.browserMcpOk", { ok: String(ok), total: String(results.length) });
}

export function formatModeSwitchHint(report: ModeSwitchReport): string {
  const parts: string[] = [];
  if (report.probe_ok === false) {
    parts.push(t("mode.probeFailShort"));
  } else if (report.probe_ok === true) {
    parts.push(t("mode.probeOk"));
  } else {
    parts.push(t("mode.switchDone"));
  }
  const applied = report.runtimes.filter((r) => r.applied).length;
  const needRestart = report.runtimes.filter(
    (r) => r.applied && (r.effector === "restart_gateway" || r.effector === "manual_restart"),
  ).length;
  if (needRestart > 0) {
    parts.push(t("mode.effectorHint", { count: String(needRestart), applied: String(applied) }));
  }
  if (report.warnings?.length) {
    parts.push(t("mode.warnings", { count: String(report.warnings.length) }));
  }
  const mcpHint = formatBrowserMcpHint(report);
  if (mcpHint) {
    parts.push(mcpHint);
  }
  return parts.join(" · ");
}

export function formatModeSwitchDetail(report: ModeSwitchReport): string {
  const bits = [report.message];
  if (report.probe_detail) bits.push(report.probe_detail);
  if (report.warnings?.length) bits.push(...report.warnings);
  return bits.filter(Boolean).join("\n");
}
