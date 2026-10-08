import type { WorkspaceCheck, WorkspaceDoctorReport } from "./types";

/** Workspace doctor checks that mean Codex was not launched from Agent Doctor. */
export const CODEX_LAUNCH_GUIDE_CHECK_IDS = new Set<string>([
  "workspace.codex.home",
  "workspace.codex.global_memory",
  "workspace.cwd.mismatch",
  "workspace.codex.shared_global_home",
]);

export function workspaceCheckNeedsCodexLaunchGuide(check: WorkspaceCheck): boolean {
  return check.status !== "pass" && CODEX_LAUNCH_GUIDE_CHECK_IDS.has(check.id);
}

export function workspaceReportNeedsCodexLaunchGuide(report: WorkspaceDoctorReport): boolean {
  return report.checks.some(workspaceCheckNeedsCodexLaunchGuide);
}
