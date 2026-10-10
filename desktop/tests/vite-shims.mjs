/**
 * Node load hook for tests that import modules built for Vite.
 *
 * `import.meta.glob` exists only inside the Vite bundle. Under Node it is
 * undefined and the module throws at import time. Importing this file first
 * turns every glob into an empty map: brand icons fall back to their letter,
 * which is all a test needs.
 *
 *   await import("./vite-shims.mjs");
 *
 * Uses module.registerHooks where Node has it (22.15+), else module.register.
 */

import * as nodeModule from "node:module";

const GLOB_CALL = /import\.meta\.glob\(\s*("[^"]*"|'[^']*'|`[^`]*`)\s*(,\s*\{[^}]*\}\s*)?\)/g;

function patch(url, result) {
  if (!url.startsWith("file:") || url.includes("/node_modules/")) return result;
  if (result.source == null) return result;
  const source = String(result.source);
  if (!source.includes("import.meta.glob")) return result;
  return { ...result, source: source.replace(GLOB_CALL, "({})") };
}

export async function load(url, context, nextLoad) {
  return patch(url, await nextLoad(url, context));
}

if (typeof nodeModule.registerHooks === "function") {
  nodeModule.registerHooks({ load: (url, context, nextLoad) => patch(url, nextLoad(url, context)) });
} else {
  nodeModule.register(import.meta.url);
}
