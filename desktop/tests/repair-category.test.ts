/**
 * The check list must show the evidence class (待验证 / 加固建议) beside the
 * status, and answer "why is this still ok" with the credential source — the
 * Hermes misjudgment was a static gap rendered like a fault.
 * Run from desktop/: node --import tsx tests/repair-category.test.ts
 */

import assert from "node:assert/strict";

const storage = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  writable: true,
  value: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => {
      storage.set(key, value);
    },
    removeItem: (key: string) => {
      storage.delete(key);
    },
  },
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  writable: true,
  value: { language: "en" },
});

const { renderRepairPreview } = await import("../src/repair-ui");
type RepairPreviewResponse =
  import("../src/types").RepairPreviewResponse;

const base: RepairPreviewResponse = {
  runtime_id: "hermes",
  display_name: "Hermes",
  summary: { pass: 1, warn: 2, fail: 0, not_applicable: 0, not_checked: 0 },
  checks: [
    {
      id: "hermes.api_key.configured",
      title: "Hermes API key configured",
      status: "warn",
      category: "unverified",
      message: "CUSTOM_API_KEY was not found in ~/.hermes/.env or the process environment",
      details: [],
    },
    {
      id: "hermes.env.permissions:/home/x/.hermes/.env",
      title: "Hermes .env permissions",
      status: "warn",
      category: "hardening",
      message: ".env permissions are 644; recommended 600",
      details: [],
    },
    {
      id: "hermes.env.parse",
      title: "Hermes .env parse",
      status: "pass",
      message: "Hermes .env contains valid KEY=value entries",
      details: [],
    },
  ],
  credential_source: "process_env",
  plan_summary: "no actions",
  suggested_repairs: [],
  can_apply_repair: false,
  backup_ids: [],
  last_execute: null,
};

const html = renderRepairPreview(base);

assert.ok(html.includes("repair-check-tag unverified"), "unverified badge renders");
assert.ok(html.includes("Unverified"), "unverified label");
assert.ok(html.includes("repair-check-tag hardening"), "hardening badge renders");
assert.ok(html.includes("Hardening"), "hardening label");
// The pass row carries no badge — pass already says it.
assert.equal(
  html.match(/repair-check-tag/g)?.length,
  2,
  "only the two warn rows carry badges",
);

// The credential source answers why the pass row is still fine.
assert.ok(
  html.includes("Credential source: process environment"),
  `credential source line renders: ${html.slice(0, 0)}`,
);
assert.ok(html.includes("repair-check-cred"), "credential source element");

// The live probe row invites the runtime-evidence step.
assert.ok(html.includes('data-action="live-probe"'), "live probe button renders");
assert.ok(html.includes("data-live-probe-slot"), "live probe slot renders");

console.log("repair category ui ok");
