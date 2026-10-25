/**
 * Model suggestions shared by home wiring, diagnose, and Ask composer.
 * Single source of truth — keep in sync with PROVIDER_PRESETS consumers.
 *
 * Each preset lists current vendor-recommended chat IDs (Oct 2026), not examples.
 * Order: latest recommended → stronger → lighter / faster.
 */

/** Fallback when protocol is known but no URL preset matched (custom endpoint). */
export const CUSTOM_MODELS_BY_PROTOCOL: Record<"openai" | "anthropic", string[]> = {
  openai: ["deepseek-flash", "gpt-5.6-terra", "gemini-3.8-flash"],
  anthropic: ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"],
};

/**
 * Menu text when the API id hides the product name.
 * The saved value stays the API id. DeepSeek calls V4.1-Flash `deepseek-flash`.
 */
const MODEL_OPTION_LABEL: Record<string, string> = {
  "deepseek-flash": "deepseek-flash（V4.1-Flash）",
};

export function modelOptionLabel(modelId: string): string {
  return MODEL_OPTION_LABEL[modelId] ?? modelId;
}

export const PRESET_MODELS_BY_ID: Record<string, string[]> = {
  // https://api-docs.deepseek.com/quick_start/pricing — deepseek-flash is V4.1-Flash.
  deepseek: ["deepseek-flash", "deepseek-v4-pro"],
  // https://www.alibabacloud.com/help/en/model-studio/models
  qwen: ["qwen3.8-max", "qwen3.7-plus", "qwen3.8-flash"],
  // https://docs.z.ai — FlashX is the faster multimodal tier. Dots, not dashes.
  glm: ["glm-5.3", "glm-5.3-flash", "glm-5.3-flashx", "glm-5.2"],
  // https://platform.minimax.io/docs — API enum is MiniMax-M3.1-Flash and MiniMax-M3.
  minimax: ["MiniMax-M3.1-Flash", "MiniMax-M3", "MiniMax-M2.5"],
  // https://platform.kimi.ai — K2.5 and kimi-latest are discontinued.
  moonshot: ["kimi-k3.1-pro", "kimi-k3.1-pro-highspeed", "kimi-k3", "kimi-k2.6"],
  // https://developers.openai.com/api/docs/models
  openai: ["gpt-5.6-terra", "gpt-5.6-sol", "gpt-5.6-luna"],
  // https://platform.claude.com/docs — Sonnet / Opus / Haiku 5.5.
  anthropic: ["claude-sonnet-5-5", "claude-opus-5-5", "claude-haiku-5-5"],
  // https://ai.google.dev/gemini-api/docs/models
  gemini: ["gemini-3.8-flash", "gemini-3.1-pro-preview", "gemini-3.6-flash"],
  // 火山方舟。evolving 指向最新豆包；deepseek-v4-1-flash-261017 是方舟上的 V4.1-Flash。
  volcengine: [
    "doubao-seed-evolving",
    "doubao-seed-2-1-pro-260915",
    "doubao-seed-2-1-lite-260915",
    "deepseek-v4-1-flash-261017",
  ],
  // https://cloud.baidu.com/doc/qianfan/s/rmh4stp0j — 文心 5.1，DeepSeek 用千帆自己的接入点。
  qianfan: ["ernie-5.1", "ernie-5.0", "deepseek-v4.1-flash"],
  siliconflow: [
    "deepseek-ai/DeepSeek-V4.1-Flash",
    "Qwen/Qwen3.8-2.4T-A95B",
    "moonshotai/Kimi-K3",
  ],
  openrouter: [
    "anthropic/claude-sonnet-5-5",
    "google/gemini-3.8-flash",
    "openai/gpt-5.6-sol",
  ],
  groq: ["openai/gpt-oss-120b", "qwen/qwen3.8-27b", "llama-3.3-70b-versatile"],
};

/** Short chip labels shown in Ask composer (mirrors wiring chips). */
const PRESET_CHIPS_BY_ID: Record<string, string> = {
  deepseek: "DeepSeek",
  qwen: "Qwen",
  glm: "GLM",
  minimax: "MiniMax",
  moonshot: "Kimi",
  openai: "ChatGPT",
  anthropic: "Claude",
  gemini: "Gemini",
  volcengine: "火山方舟",
  qianfan: "千帆",
  siliconflow: "硅基流动",
  openrouter: "OpenRouter",
  groq: "Groq",
};

const PRESET_URL_MATCHERS: Array<{ match: RegExp; id: string }> = [
  { match: /api\.deepseek\.com/i, id: "deepseek" },
  { match: /dashscope/i, id: "qwen" },
  { match: /bigmodel\.cn|api\.z\.ai/i, id: "glm" },
  { match: /minimax\.cn|minimaxi\.com|minimax\.io/i, id: "minimax" },
  { match: /moonshot\.(cn|ai)/i, id: "moonshot" },
  { match: /api\.openai\.com/i, id: "openai" },
  { match: /api\.anthropic\.com/i, id: "anthropic" },
  { match: /generativelanguage\.googleapis\.com/i, id: "gemini" },
  { match: /volces\.com/i, id: "volcengine" },
  { match: /qianfan\.baidubce\.com/i, id: "qianfan" },
  { match: /siliconflow\.(cn|com)/i, id: "siliconflow" },
  { match: /openrouter\.ai/i, id: "openrouter" },
  { match: /api\.groq\.com/i, id: "groq" },
];

export function modelsForPresetId(presetId: string): string[] {
  const list = PRESET_MODELS_BY_ID[presetId];
  return list ? [...list] : [];
}

export function modelsForCustomProtocol(protocol: string): string[] {
  return protocol === "anthropic"
    ? [...CUSTOM_MODELS_BY_PROTOCOL.anthropic]
    : [...CUSTOM_MODELS_BY_PROTOCOL.openai];
}

function presetIdForUrl(url: string): string | null {
  return PRESET_URL_MATCHERS.find((row) => row.match.test(url))?.id ?? null;
}

/**
 * Merge preset defaults with live `/models` ids from verify.
 * Keep current selection first; then curated defaults; then the rest of live sample.
 */
export function mergeLiveModels(
  presetOrUrlModels: string[],
  liveSample: string[],
  currentModel?: string | null,
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  const push = (id: string) => {
    const trimmed = id.trim();
    if (!trimmed || seen.has(trimmed)) return;
    seen.add(trimmed);
    out.push(trimmed);
  };
  const current = currentModel?.trim();
  if (current) push(current);
  for (const id of presetOrUrlModels) push(id);
  for (const id of liveSample) push(id);
  return out;
}

/** Build selectable model list for a wired provider URL + current model. */
export function modelsForProviderUrl(url: string, currentModel?: string | null): string[] {
  const id = presetIdForUrl(url);
  const list = id ? modelsForPresetId(id) : [];
  return mergeLiveModels(list, [], currentModel);
}

/** Compact provider label for Ask composer (chip when known, else stored name). */
export function providerChipForUrl(url: string, fallbackName: string): string {
  const id = presetIdForUrl(url);
  if (id && PRESET_CHIPS_BY_ID[id]) return PRESET_CHIPS_BY_ID[id];
  const name = fallbackName.trim();
  return name || "Provider";
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Short label for pickers when the provider chip is already visible nearby. */
export function shortModelLabel(modelId: string, presetId?: string | null): string {
  let raw = modelId.trim();
  if (!raw) {
    return "—";
  }
  if (raw.includes("/")) {
    raw = raw.slice(raw.lastIndexOf("/") + 1);
  }
  const prefixes: string[] = [];
  if (presetId) {
    prefixes.push(presetId);
    const chip = PRESET_CHIPS_BY_ID[presetId];
    if (chip) {
      prefixes.push(chip);
    }
  }
  for (const prefix of prefixes) {
    const re = new RegExp(`^${escapeRegExp(prefix)}[-_.\\s]*`, "i");
    if (re.test(raw) && raw.length > prefix.length) {
      raw = raw.replace(re, "");
      break;
    }
  }
  raw = raw.replace(/^[-_.\s]+/, "").trim();
  if (!raw) {
    return modelId.trim();
  }
  return raw
    .split(/[-_]+/)
    .filter(Boolean)
    .map((part) => {
      if (/^v?\d/i.test(part)) {
        return part.toUpperCase();
      }
      return part.charAt(0).toUpperCase() + part.slice(1);
    })
    .join(" ");
}
