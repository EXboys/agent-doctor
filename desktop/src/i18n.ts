import { en, type MessageKey } from "./i18n/en";
import { zh } from "./i18n/zh";

export type Locale = "en" | "zh";
export type { MessageKey };

const STORAGE_KEY = "agent-doctor-locale";

const messages = {
  en,
  zh,
} as const;

let locale: Locale = detectLocale();

function detectLocale(): Locale {
  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved === "en" || saved === "zh") {
    return saved;
  }
  const lang = navigator.language.toLowerCase();
  return lang.startsWith("zh") ? "zh" : "en";
}

export function getLocale(): Locale {
  return locale;
}

export function setLocale(next: Locale): void {
  locale = next;
  localStorage.setItem(STORAGE_KEY, next);
  document.documentElement.lang = next === "zh" ? "zh-CN" : "en";
}

export function t(key: MessageKey, params?: Record<string, string>): string {
  let text: string = messages[locale][key] ?? messages.en[key] ?? key;
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
  const localized = messages[locale] as Record<string, string>;
  const english = messages.en as Record<string, string>;
  return localized[key] ?? english[key] ?? "";
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
