import type { MessageKey } from "./i18n/en";

export type Locale = "en" | "zh";
export type { MessageKey };

type Dict = Record<MessageKey, string>;

const STORAGE_KEY = "agent-doctor-locale";

let locale: Locale = detectLocale();
// Fixed at boot so `t()` always has a dictionary to fall back to while a
// switched-to locale is still streaming in.
const bootLocale = locale;

/** Only the active locale is fetched per window; the other stays a lazy chunk. */
const loaded: Partial<Record<Locale, Dict>> = {};

function detectLocale(): Locale {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved === "en" || saved === "zh") {
    return saved;
  }
  const lang = navigator.language.toLowerCase();
  return lang.startsWith("zh") ? "zh" : "en";
}

async function loadDict(which: Locale): Promise<Dict> {
  // Literal specifiers: Rollup splits each dictionary into its own lazy chunk.
  if (which === "zh") {
    const mod = await import("./i18n/zh");
    return mod.zh;
  }
  const mod = await import("./i18n/en");
  return mod.en;
}

function ensureLoaded(which: Locale): Promise<void> {
  return loadDict(which).then((dict) => {
    loaded[which] = dict;
  });
}

// Every entry script is a module importing this file, so the active
// dictionary is ready before the first render; the other locale never loads.
await ensureLoaded(locale);

export function getLocale(): Locale {
  return locale;
}

export async function setLocale(next: Locale): Promise<void> {
  locale = next;
  localStorage.setItem(STORAGE_KEY, next);
  document.documentElement.lang = next === "zh" ? "zh-CN" : "en";
  await ensureLoaded(next);
}

export function t(key: MessageKey, params?: Record<string, string>): string {
  let text: string =
    loaded[locale]?.[key] ?? loaded[bootLocale]![key] ?? key;
  if (params) {
    for (const [name, value] of Object.entries(params)) {
      text = text.split(`{${name}}`).join(value);
    }
  }
  return text;
}

/** Fill `{name}` (and other `{param}`) in copy — never leave raw placeholders on screen. */
export function tNamed(key: MessageKey, name: string, params?: Record<string, string>): string {
  const merged = { name: name.trim() || "—", ...params };
  let text = t(key, merged);
  for (const [param, value] of Object.entries(merged)) {
    text = text.split(`{${param}}`).join(value);
  }
  return text;
}

/** Blurb keyed by runtime id (`resources.agentBlurb.<id>` in en.ts / zh.ts). */
export function tRuntimeBlurb(runtimeId: string): string {
  const key = `resources.agentBlurb.${runtimeId}`;
  const localized = loaded[locale] as Record<string, string> | undefined;
  const boot = loaded[bootLocale] as Record<string, string>;
  return localized?.[key] ?? boot[key] ?? "";
}

export function applyStaticI18n(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>("[data-i18n]").forEach((element) => {
    const key = element.dataset.i18n as MessageKey | undefined;
    if (!key) {
      return;
    }
    element.textContent = t(key);
  });

  root.querySelectorAll<HTMLElement>("[data-i18n-title]").forEach((element) => {
    const key = element.dataset.i18nTitle as MessageKey | undefined;
    if (!key) {
      return;
    }
    element.title = t(key);
  });

  const presetTrigger = document.querySelector<HTMLButtonElement>("#preset-trigger");
  if (presetTrigger) {
    presetTrigger.setAttribute(
      "aria-label",
      locale === "zh" ? "配置预设" : "Profile preset",
    );
  }
}
