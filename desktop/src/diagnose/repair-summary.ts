import { t } from "../i18n";
import type { DeepRepairSummary, RepairCheckChange } from "../ipc";
import { plainRepairCheckCopy } from "../repair-plain";

function plainTitles(checks: RepairCheckChange[], status?: string): string[] {
  const generic = new Set([t("repair.check.genericOkTitle"), t("repair.check.genericBadTitle")]);
  const titles = checks
    .map((check) => plainRepairCheckCopy({ ...check, status: status ?? check.status }).title)
    .filter((title) => !generic.has(title));
  return [...new Set(titles)];
}

function list(titles: string[], total: number): string {
  const sep = t("diagnose.flow.repairListSep");
  const named = titles.join(sep);
  const rest = total - titles.length;
  if (rest <= 0) {
    return named;
  }
  const more = t("diagnose.flow.repairListMore", { count: String(rest) });
  return named ? `${named}${sep}${more}` : more;
}

/** One plain sentence (or two) saying what a deep repair changed. */
export function describeRepairSummary(summary: DeepRepairSummary): string {
  const parts: string[] = [];
  if (summary.fixed.length > 0) {
    parts.push(
      t("diagnose.flow.repairFixedList", {
        items: list(plainTitles(summary.fixed, "pass"), summary.fixed.length),
      }),
    );
  }
  if (summary.new_issues.length > 0) {
    parts.push(
      t("diagnose.flow.repairNewIssues", {
        items: list(plainTitles(summary.new_issues), summary.new_issues.length),
      }),
    );
  }
  if (summary.rolled_back_issues.length > 0) {
    parts.push(
      t("diagnose.flow.repairRolledBack", {
        items: list(plainTitles(summary.rolled_back_issues), summary.rolled_back_issues.length),
      }),
    );
  }
  if (parts.length > 0) {
    return parts.join(" ");
  }
  return summary.executed.length > 0
    ? t("diagnose.flow.repairNoGain", { count: String(summary.executed.length) })
    : t("diagnose.flow.repairNothing");
}
