import type { AskRuntime } from "../ask-resources";

/** Survives Ask soft-reload; beats the create-time `__AD_ASK_RUNTIME__` init script. */
export const ASK_PENDING_RUNTIME_KEY = "ad.ask.pendingRuntime";

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
