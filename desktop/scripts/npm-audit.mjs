#!/usr/bin/env node
// npm-audit policy for desktop/, mirroring .cargo/audit.toml.
//
// Production dependencies (`npm audit --omit=dev`) allow zero high/critical
// advisories. Dev-only advisories may be listed in IGNORED_DEV_ADVISORIES only
// while upstream has no non-breaking fix; every entry documents the chain and
// fails automatically as soon as `npm audit fix` (without `--force`) can clear
// it.

import { appendFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const IGNORED_DEV_ADVISORIES = {
  // picomatch <= 4.0.7 ReDoS on crafted extglobs (+(*) / +(*a)).
  // Reaches desktop/ only in devDependencies: vite -> tinyglobby -> fdir ->
  // picomatch, where globs come from local build config and never ship in the
  // Tauri app. Upstream has no patched release > 4.0.7 yet
  // (first_patched_version: null); `npm audit fix --force` proposes
  // downgrading vite 8 -> 6.2.7 only because vite <= 6.2 bundled picomatch
  // inline instead of declaring it in dependencies.
  // Drop once picomatch ships a patched release.
  "GHSA-fc5q-39xc-vjpf": {
    package: "picomatch",
    reason:
      "build-time only via vite -> tinyglobby -> fdir; upstream picomatch has no release > 4.0.7 yet (force-fix downgrades vite 8 -> 6.2.7, which merely bundles picomatch inline)",
  },
};

const HIGH_SEVERITIES = new Set(["high", "critical"]);
const GHSA_RE = /GHSA-[0-9a-z]{4}-[0-9a-z]{4}-[0-9a-z]{4}/i;

export function hasNonBreakingFix(fixAvailable) {
  if (fixAvailable === true) return true;
  if (typeof fixAvailable === "object" && fixAvailable !== null) {
    return fixAvailable.isSemVerMajor === false;
  }
  return false;
}

function collectRootAdvisories(pkgName, vulnerabilities, visited = new Set()) {
  if (visited.has(pkgName)) return [];
  visited.add(pkgName);
  const vuln = vulnerabilities[pkgName];
  if (!vuln || !Array.isArray(vuln.via)) return [];

  const advisories = [];
  for (const item of vuln.via) {
    if (typeof item === "string") {
      advisories.push(...collectRootAdvisories(item, vulnerabilities, visited));
    } else if (typeof item === "object" && item !== null) {
      advisories.push({ ownerPackage: pkgName, ownerVuln: vuln, advisory: item });
    }
  }
  return advisories;
}

export function evaluateAuditReport(report, allowedAdvisories = {}) {
  const failures = [];
  const ignoredHits = new Map();
  const staleIgnores = [];

  if (!report || typeof report !== "object") {
    return {
      ok: false,
      failures: ["npm audit did not return a JSON object"],
      ignoredHits: [],
      staleIgnores: [],
    };
  }

  if (report.error) {
    const summary =
      report.error.summary || report.error.detail || JSON.stringify(report.error);
    return {
      ok: false,
      failures: [`npm audit returned an error: ${summary}`],
      ignoredHits: [],
      staleIgnores: [],
    };
  }

  const vulnerabilities = report.vulnerabilities;
  if (!vulnerabilities || typeof vulnerabilities !== "object") {
    return {
      ok: false,
      failures: ["npm audit JSON is missing the 'vulnerabilities' map"],
      ignoredHits: [],
      staleIgnores: [],
    };
  }

  for (const [pkgName, vuln] of Object.entries(vulnerabilities)) {
    if (!vuln || typeof vuln !== "object") continue;
    const pkgHigh = HIGH_SEVERITIES.has(vuln.severity);
    const roots = collectRootAdvisories(pkgName, vulnerabilities);
    const highRoots = roots.filter(({ advisory }) =>
      HIGH_SEVERITIES.has(advisory.severity),
    );

    if (pkgHigh && highRoots.length === 0) {
      failures.push(
        `${pkgName}: reported ${vuln.severity} severity without a root advisory detail`,
      );
      continue;
    }

    for (const { ownerPackage, ownerVuln, advisory } of highRoots) {
      const ghsaMatch =
        typeof advisory.url === "string" ? advisory.url.match(GHSA_RE) : null;
      const ghsaId = ghsaMatch ? ghsaMatch[0] : null;
      const rule =
        ghsaId && Object.hasOwn(allowedAdvisories, ghsaId)
          ? allowedAdvisories[ghsaId]
          : null;

      if (!rule) {
        failures.push(
          `${pkgName} (via ${ownerPackage}): unignored ${advisory.severity} advisory ${ghsaId ?? advisory.url ?? "unknown"} — ${advisory.title ?? "no title"}`,
        );
        continue;
      }

      if (rule.package && rule.package !== ownerPackage) {
        failures.push(
          `${ghsaId}: expected owner package '${rule.package}', got '${ownerPackage}'`,
        );
        continue;
      }

      if (hasNonBreakingFix(ownerVuln.fixAvailable) || hasNonBreakingFix(vuln.fixAvailable)) {
        failures.push(
          `${ghsaId} (${ownerPackage}) now has a non-breaking upstream fix — run 'npm audit fix' in desktop/ and remove ${ghsaId} from IGNORED_DEV_ADVISORIES`,
        );
        continue;
      }

      if (!ignoredHits.has(ghsaId)) {
        ignoredHits.set(ghsaId, {
          ghsaId,
          package: ownerPackage,
          severity: advisory.severity,
          title: advisory.title ?? "",
          url: advisory.url ?? "",
          reason: rule.reason,
        });
      }
    }
  }

  for (const ghsaId of Object.keys(allowedAdvisories)) {
    if (!ignoredHits.has(ghsaId)) {
      staleIgnores.push(ghsaId);
    }
  }

  return {
    ok: failures.length === 0,
    failures: [...new Set(failures)],
    ignoredHits: [...ignoredHits.values()],
    staleIgnores,
  };
}

function runNpmAuditJson(desktopDir, extraArgs) {
  const npmCmd = process.platform === "win32" ? "npm.cmd" : "npm";
  const result = spawnSync(npmCmd, ["audit", "--json", ...extraArgs], {
    cwd: desktopDir,
    encoding: "utf8",
    shell: process.platform === "win32",
  });

  if (result.error) {
    throw new Error(`failed to spawn ${npmCmd}: ${result.error.message}`);
  }
  const raw = (result.stdout || "").trim();
  if (!raw) {
    throw new Error(
      `npm audit produced no JSON output (exit ${result.status}): ${(result.stderr || "").trim()}`,
    );
  }
  try {
    return JSON.parse(raw);
  } catch (err) {
    throw new Error(
      `failed to parse npm audit JSON: ${err.message}\n${raw.slice(0, 500)}`,
    );
  }
}

function writeSummary(lines) {
  const text = `${lines.join("\n")}\n`;
  process.stdout.write(text);
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, text, "utf8");
  }
}

function main() {
  const desktopDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const lines = ["### npm audit (`desktop`)", ""];

  // 1. Production dependencies shipped in the Tauri bundle: zero exceptions.
  const prodReport = runNpmAuditJson(desktopDir, ["--omit=dev"]);
  const prodResult = evaluateAuditReport(prodReport, {});
  if (!prodResult.ok) {
    lines.push("Production dependencies (`--omit=dev`) failed audit:");
    for (const failure of prodResult.failures) {
      lines.push(`- ${failure}`);
    }
    writeSummary(lines);
    process.exit(1);
  }
  lines.push("- Production dependencies (`--omit=dev`): 0 high/critical vulnerabilities.");

  // 2. Full dependency tree (including devDependencies): enforce policy.
  const fullReport = runNpmAuditJson(desktopDir, []);
  const fullResult = evaluateAuditReport(fullReport, IGNORED_DEV_ADVISORIES);

  for (const hit of fullResult.ignoredHits) {
    lines.push(
      `- Ignored dev advisory [${hit.ghsaId}](${hit.url}) (${hit.package}, ${hit.severity}): ${hit.reason}`,
    );
  }

  for (const ghsaId of fullResult.staleIgnores) {
    const msg = `${ghsaId} is no longer reported by npm audit; remove it from desktop/scripts/npm-audit.mjs`;
    console.warn(`::warning::${msg}`);
    lines.push(`- Warning: ${msg}`);
  }

  if (!fullResult.ok) {
    lines.push("", "Unresolved high/critical advisories:");
    for (const failure of fullResult.failures) {
      lines.push(`- ${failure}`);
    }
    writeSummary(lines);
    process.exit(1);
  }

  lines.push("- Full tree policy check passed.");
  writeSummary(lines);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (err) {
    console.error(err instanceof Error ? err.message : String(err));
    process.exit(1);
  }
}
