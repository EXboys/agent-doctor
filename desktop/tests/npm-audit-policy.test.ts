import assert from "node:assert/strict";
import {
  IGNORED_DEV_ADVISORIES,
  evaluateAuditReport,
} from "../scripts/npm-audit.mjs";

// 1. Clean report passes and flags unused ignore entries as non-fatal warnings.
{
  const res = evaluateAuditReport({ vulnerabilities: {} }, IGNORED_DEV_ADVISORIES);
  assert.equal(res.ok, true);
  assert.deepEqual(res.failures, []);
  assert.ok(res.staleIgnores.includes("GHSA-fc5q-39xc-vjpf"));
}

// 2. Unpatched picomatch dev chain (force-fix downgrades vite 8 -> 6.2.7) passes
// under IGNORED_DEV_ADVISORIES, and fails under empty production policy.
const picomatchUnpatchedReport = {
  auditReportVersion: 2,
  vulnerabilities: {
    picomatch: {
      name: "picomatch",
      severity: "high",
      via: [
        {
          name: "picomatch",
          title: "picomatch vulnerable to ReDoS through single-branch repeated extglobs",
          url: "https://github.com/advisories/GHSA-fc5q-39xc-vjpf",
          severity: "high",
          range: "<=4.0.7",
        },
      ],
      fixAvailable: { name: "vite", version: "6.2.7", isSemVerMajor: true },
    },
    fdir: {
      name: "fdir",
      severity: "high",
      via: ["picomatch"],
      fixAvailable: { name: "vite", version: "6.2.7", isSemVerMajor: true },
    },
    tinyglobby: {
      name: "tinyglobby",
      severity: "high",
      via: ["fdir", "picomatch"],
      fixAvailable: { name: "vite", version: "6.2.7", isSemVerMajor: true },
    },
    vite: {
      name: "vite",
      severity: "high",
      via: ["picomatch", "tinyglobby"],
      fixAvailable: { name: "vite", version: "6.2.7", isSemVerMajor: true },
    },
  },
};

{
  const devRes = evaluateAuditReport(picomatchUnpatchedReport, IGNORED_DEV_ADVISORIES);
  assert.equal(devRes.ok, true);
  assert.equal(devRes.ignoredHits.length, 1);
  assert.equal(devRes.ignoredHits[0].ghsaId, "GHSA-fc5q-39xc-vjpf");
  assert.deepEqual(devRes.staleIgnores, []);

  const prodRes = evaluateAuditReport(picomatchUnpatchedReport, {});
  assert.equal(prodRes.ok, false);
  assert.ok(prodRes.failures.some((f) => f.includes("GHSA-fc5q-39xc-vjpf")));
}

// 3. As soon as upstream publishes a non-breaking fix, the ignore expires and fails.
{
  const fixable = structuredClone(picomatchUnpatchedReport);
  fixable.vulnerabilities.picomatch.fixAvailable = true;
  const res = evaluateAuditReport(fixable, IGNORED_DEV_ADVISORIES);
  assert.equal(res.ok, false);
  assert.ok(res.failures.some((f) => f.includes("non-breaking upstream fix")));
}

// 4. A second unignored high advisory on vite still fails even alongside picomatch.
{
  const mixed = structuredClone(picomatchUnpatchedReport);
  mixed.vulnerabilities.vite.via.push({
    name: "vite",
    title: "Hypothetical dev server fs bypass",
    url: "https://github.com/advisories/GHSA-1111-2222-3333",
    severity: "high",
    range: "<8.3.9",
  });
  const res = evaluateAuditReport(mixed, IGNORED_DEV_ADVISORIES);
  assert.equal(res.ok, false);
  assert.ok(res.failures.some((f) => f.includes("GHSA-1111-2222-3333")));
}

// 5. Registry / malformed JSON errors fail closed.
{
  const res = evaluateAuditReport({
    error: { code: "ENOAUDIT", summary: "audit endpoint unavailable" },
  });
  assert.equal(res.ok, false);
  assert.ok(res.failures[0].includes("audit endpoint unavailable"));
}

// 6. Moderate/low advisories do not fail the high/critical gate.
{
  const moderateOnly = {
    vulnerabilities: {
      example: {
        name: "example",
        severity: "moderate",
        via: [
          {
            name: "example",
            title: "Moderate issue",
            url: "https://github.com/advisories/GHSA-aaaa-bbbb-cccc",
            severity: "moderate",
          },
        ],
        fixAvailable: false,
      },
    },
  };
  const res = evaluateAuditReport(moderateOnly, IGNORED_DEV_ADVISORIES);
  assert.equal(res.ok, true);
}
