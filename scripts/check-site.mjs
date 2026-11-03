#!/usr/bin/env node
// Static checks for the marketing site (site/). No dependencies; run:
//   node scripts/check-site.mjs
//
// Guards the classes of mistakes the site already shipped once:
// - a css/js content change deployed next to HTML still pointing at the old
//   cache key (the edge serves the mismatched pair for hours) — every
//   stampable reference must carry ?v= and the hash must match the content;
// - a renamed or deleted page/asset still linked from HTML (a download or
//   nav click lands on Cloudflare's 404);
// - a _redirects rule whose 200 target does not exist;
// - site JS that no longer parses (it ships to browsers untransformed).
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync, existsSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const siteDir = join(dirname(fileURLToPath(import.meta.url)), "..", "site");
const problems = [];

function fail(message) {
  problems.push(message);
}

// --- 1. Site JS must parse (browsers get it exactly as committed). ---------
for (const name of readdirSync(join(siteDir, "js"))) {
  if (!name.endsWith(".js")) continue;
  const file = join(siteDir, "js", name);
  const result = spawnSync(process.execPath, ["--check", file], { encoding: "utf8" });
  if (result.status !== 0) {
    fail(`site/js/${name} does not parse:\n${result.stderr.trim()}`);
  }
}

// --- 2. Stampable asset references must carry the current content hash. ----
const stampables = ["css/site.css", "js/nav.js", "js/site.js", "js/skills.js"];
const expectedHash = createHash("sha256");
for (const rel of stampables) {
  expectedHash.update(readFileSync(join(siteDir, rel)));
}
const stamp = expectedHash.digest("hex").slice(0, 8);

for (const page of readdirSync(siteDir).filter((name) => name.endsWith(".html"))) {
  const html = readFileSync(join(siteDir, page), "utf8");
  const refRe = /(?:href|src)="(\/(?:css|js)\/[^"]+)"/g;
  for (const match of html.matchAll(refRe)) {
    const ref = match[1];
    if (!stampables.some((rel) => ref === `/${rel}` || ref.startsWith(`/${rel}?`))) continue;
    const stamped = ref.match(/\?v=([0-9a-f]{8})$/);
    if (!stamped) {
      fail(`${page} references ${ref} without ?v= — run scripts/site-version.sh after changing css/js`);
    } else if (stamped[1] !== stamp) {
      fail(
        `${page} references ${ref} stamped ${stamped[1]} but the content hashes to ${stamp}` +
          " — run scripts/site-version.sh",
      );
    }
  }
}

// --- 3. Local links and images must resolve to committed files. ------------
function resolveLocal(path) {
  const [raw, fragment] = path.split("#");
  if (raw === "/" || raw === "") return { file: "index.html", fragment };
  const clean = raw.split("?")[0].replace(/\/+$/, "");
  if (clean === "") return { file: "index.html", fragment };
  for (const candidate of [clean.replace(/^\//, ""), `${clean.replace(/^\//, "")}.html`]) {
    const file = join(siteDir, candidate);
    if (existsSync(file) && statSync(file).isFile()) return { file: candidate, fragment };
  }
  return null;
}

for (const page of readdirSync(siteDir).filter((name) => name.endsWith(".html"))) {
  const html = readFileSync(join(siteDir, page), "utf8");
  const refRe = /(?:href|src)="([^"]+)"/g;
  for (const match of html.matchAll(refRe)) {
    const ref = match[1];
    if (!ref.startsWith("/")) continue; // anchors, external URLs, protocol-relative
    const target = resolveLocal(ref);
    if (!target) {
      fail(`${page} points at ${ref} which does not exist in site/`);
      continue;
    }
    if (target.fragment) {
      const targetHtml = readFileSync(join(siteDir, target.file), "utf8");
      if (!targetHtml.includes(`id="${target.fragment}"`)) {
        fail(`${page} points at ${ref} but ${target.file} has no id="${target.fragment}"`);
      }
    }
  }
}

// --- 4. _redirects targets: 200 rules need the file, 302 rules a URL. ------
const redirects = readFileSync(join(siteDir, "_redirects"), "utf8");
for (const [index, rawLine] of redirects.split("\n").entries()) {
  const line = rawLine.trim();
  if (!line || line.startsWith("#")) continue;
  const parts = line.split(/\s+/);
  if (parts.length !== 3) {
    fail(`_redirects line ${index + 1} is not "from target code": ${line}`);
    continue;
  }
  const [, target, code] = parts;
  if (code === "200") {
    if (!resolveLocal(target)) fail(`_redirects line ${index + 1} serves ${target} which does not exist in site/`);
  } else if (!/^https?:\/\//.test(target)) {
    fail(`_redirects line ${index + 1} redirects to ${target} instead of an absolute URL`);
  }
}

if (problems.length > 0) {
  console.error(`site checks failed (${problems.length}):`);
  for (const problem of problems) console.error(`- ${problem}`);
  process.exit(1);
}
console.log("site checks ok");
