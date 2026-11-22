/**
 * Run from desktop/: node --import tsx tests/agent-catalog-groups.test.ts
 */
import assert from "node:assert/strict";
import {
  agentCatalogGroup,
  agentCatalogRank,
  groupAgentCatalog,
  installsDesktopAndCli,
  isExperimentalRuntime,
  splitAgentCatalog,
} from "../src/agent-catalog-groups.ts";

// Only Hermes and DeepSeek Harness are unstable/experimental.
assert.equal(isExperimentalRuntime("hermes"), true);
assert.equal(isExperimentalRuntime("deepseek-harness"), true);
assert.equal(isExperimentalRuntime("workbuddy"), false);
assert.equal(isExperimentalRuntime("qoder"), false);
assert.equal(isExperimentalRuntime("zcode"), false);
assert.equal(isExperimentalRuntime("codex"), false);
assert.equal(isExperimentalRuntime("cursor"), false);
assert.equal(isExperimentalRuntime("claude-code"), false);
assert.equal(isExperimentalRuntime("openclaw"), false);
assert.equal(isExperimentalRuntime("pi"), false);

// Dual-client suites install both CLI and desktop app; others install CLI only.
assert.equal(installsDesktopAndCli("workbuddy"), true);
assert.equal(installsDesktopAndCli("qoder"), true);
assert.equal(installsDesktopAndCli("zcode"), true);
assert.equal(installsDesktopAndCli("deepseek-harness"), false);
assert.equal(installsDesktopAndCli("codex"), false);
assert.equal(installsDesktopAndCli("claude-code"), false);
assert.equal(installsDesktopAndCli("cursor"), true);
assert.equal(installsDesktopAndCli("openclaw"), false);
assert.equal(installsDesktopAndCli("hermes"), false);

// Four groups: builtin, direct install (npm), dual-client (domestic), experimental.
assert.equal(agentCatalogGroup("doctor"), "builtin");
assert.equal(agentCatalogGroup("codex"), "npm");
assert.equal(agentCatalogGroup("claude-code"), "npm");
assert.equal(agentCatalogGroup("openclaw"), "npm");
assert.equal(agentCatalogGroup("pi"), "npm");
assert.equal(installsDesktopAndCli("pi"), false);
assert.equal(agentCatalogGroup("cursor"), "domestic");
assert.equal(agentCatalogGroup("workbuddy"), "domestic");
assert.equal(agentCatalogGroup("qoder"), "domestic");
assert.equal(agentCatalogGroup("zcode"), "domestic");
assert.equal(agentCatalogGroup("deepseek-harness"), "experimental");
assert.equal(agentCatalogGroup("hermes"), "experimental");

// Built-in ranks first, then npm-backed runtimes, dual-client suites, experimental.
assert(agentCatalogRank("doctor") < agentCatalogRank("codex"));
assert(agentCatalogRank("codex") < agentCatalogRank("workbuddy"));
assert(agentCatalogRank("claude-code") < agentCatalogRank("qoder"));
assert(agentCatalogRank("openclaw") < agentCatalogRank("cursor"));
assert(agentCatalogRank("cursor") < agentCatalogRank("workbuddy"));
assert(agentCatalogRank("workbuddy") < agentCatalogRank("deepseek-harness"));
assert(agentCatalogRank("qoder") < agentCatalogRank("hermes"));

const grouped = groupAgentCatalog([
  { id: "hermes" },
  { id: "doctor" },
  { id: "qoder" },
  { id: "codex" },
  { id: "cursor" },
  { id: "workbuddy" },
  { id: "deepseek-harness" },
  { id: "claude-code" },
  { id: "zcode" },
]);
assert.deepEqual(
  grouped.builtin.map((row) => row.id),
  ["doctor"],
);
assert.deepEqual(
  grouped.npm.map((row) => row.id),
  ["codex", "claude-code"],
);
assert.deepEqual(
  grouped.domestic.map((row) => row.id),
  ["qoder", "cursor", "workbuddy", "zcode"],
);
assert.deepEqual(
  grouped.experimental.map((row) => row.id),
  ["hermes", "deepseek-harness"],
);

const split = splitAgentCatalog([
  { id: "codex" },
  { id: "workbuddy" },
  { id: "hermes" },
  { id: "deepseek-harness" },
]);
assert.deepEqual(
  split.ready.map((row) => row.id),
  ["codex", "workbuddy"],
);
assert.deepEqual(
  split.experimental.map((row) => row.id),
  ["hermes", "deepseek-harness"],
);
