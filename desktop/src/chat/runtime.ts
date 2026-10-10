import type { AskRuntime } from "../ask-resources";

/** Survives Ask soft-reload; beats the create-time `__AD_ASK_RUNTIME__` init script. */
export const ASK_PENDING_RUNTIME_KEY = "ad.ask.pendingRuntime";

/** Ask → main: the open conversation's agent changed. */
export const ASK_ACTIVE_RUNTIME_EVENT = "ask-active-runtime";

/** Main → ask: the user picked an agent card; switch to that agent's conversation. */
export const ASK_SELECT_RUNTIME_EVENT = "ask-select-runtime";

/** Main → ask: report the open conversation's agent (ask may already be open). */
export const ASK_RUNTIME_QUERY_EVENT = "ask-runtime-query";

export function runtimeDisplayName(runtime: AskRuntime): string {
  if (runtime === "codex") return "Codex";
  if (runtime === "hermes") return "Hermes";
  if (runtime === "openclaw") return "OpenClaw";
  if (runtime === "deepseek-harness") return "DeepSeek Harness";
  return "Claude Code";
}

export function isAskRuntime(value: string | null | undefined): value is AskRuntime {
  return (
    value === "claude-code" ||
    value === "codex" ||
    value === "hermes" ||
    value === "openclaw" ||
    value === "deepseek-harness"
  );
}

export function runtimeFromLocation(): string | null {
  try {
    const pending = localStorage.getItem(ASK_PENDING_RUNTIME_KEY)?.trim();
    if (pending) {
      localStorage.removeItem(ASK_PENDING_RUNTIME_KEY);
      return pending;
    }
  } catch {
    /* ignore */
  }
  const injected = (window as Window & { __AD_ASK_RUNTIME__?: unknown }).__AD_ASK_RUNTIME__;
  if (typeof injected === "string" && injected.trim()) {
    return injected.trim();
  }
  const query = new URLSearchParams(window.location.search).get("runtime");
  if (query) {
    return query;
  }
  const hash = window.location.hash.replace(/^#/, "");
  if (!hash) {
    return null;
  }
  if (hash.startsWith("runtime=")) {
    return decodeURIComponent(hash.slice("runtime=".length).split("&")[0] ?? "");
  }
  return new URLSearchParams(hash).get("runtime");
}
