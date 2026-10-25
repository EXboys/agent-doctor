/**
 * Run from desktop/: node --import tsx tests/i18n-pairs.test.ts
 *
 * English and Chinese copy must use the same keys, and the same {placeholders}.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { en } from "../src/i18n/en.ts";
import { zh } from "../src/i18n/zh.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "../src/i18n");

function duplicateKeys(filename: string): string[] {
  const source = readFileSync(join(root, filename), "utf8");
  const counts = new Map<string, number>();
  for (const match of source.matchAll(/^\s*"([^"]+)":/gm)) {
    const key = match[1] ?? "";
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return [...counts].filter(([, count]) => count > 1).map(([key]) => key);
}

function placeholders(text: string): string[] {
  return [...text.matchAll(/\{([A-Za-z0-9_]+)\}/g)].map((match) => match[1] ?? "").sort();
}

const enText = en as Record<string, string>;
const zhText = zh as Record<string, string>;
const enKeys = Object.keys(enText).sort();
const zhKeys = Object.keys(zhText).sort();
const zhSet = new Set(zhKeys);
const enSet = new Set(enKeys);

const missingInZh = enKeys.filter((key) => !zhSet.has(key));
const missingInEn = zhKeys.filter((key) => !enSet.has(key));
assert.deepEqual(missingInZh, [], `zh is missing:\n${missingInZh.join("\n")}`);
assert.deepEqual(missingInEn, [], `en is missing:\n${missingInEn.join("\n")}`);

const duplicateEn = duplicateKeys("en.ts");
const duplicateZh = duplicateKeys("zh.ts");
assert.deepEqual(duplicateEn, [], `duplicate en keys:\n${duplicateEn.join("\n")}`);
assert.deepEqual(duplicateZh, [], `duplicate zh keys:\n${duplicateZh.join("\n")}`);

const placeholderMismatches: string[] = [];
for (const key of enKeys) {
  const left = placeholders(enText[key] ?? "");
  const right = placeholders(zhText[key] ?? "");
  if (left.join("\0") !== right.join("\0")) {
    placeholderMismatches.push(`${key}: en {${left.join("} {")}} zh {${right.join("} {")}}`);
  }
}
assert.deepEqual(
  placeholderMismatches,
  [],
  `placeholder mismatch:\n${placeholderMismatches.join("\n")}`,
);

console.log(`i18n-pairs.test.ts ok (${enKeys.length} keys)`);
