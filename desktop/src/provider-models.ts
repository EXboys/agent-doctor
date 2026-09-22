/**
 * Model suggestions shared by home wiring, diagnose, and Ask composer.
 * Single source of truth — keep in sync with PROVIDER_PRESETS consumers.
 *
 * Each preset lists current vendor-recommended chat IDs (Sep 2026), not examples.
 * Order: balanced default → stronger → lighter / faster.
 */

/** Fallback when protocol is known but no URL preset matched (custom endpoint). */
export const CUSTOM_MODELS_BY_PROTOCOL: Record<"openai" | "anthropic", string[]> = {
  openai: ["deepseek-flash", "gpt-5.6-terra", "gemini-3.8-flash"],
  anthropic: ["claude-sonnet-5", "claude-opus-5", "claude-haiku-4-5"],
};

export const PRESET_MODELS_BY_ID: Record<string, string[]> = {
  // https://api-docs.deepseek.com/quick_start/pricing
  deepseek: ["deepseek-flash", "deepseek-v4-pro", "deepseek-v4-flash"],
  // https://www.alibabacloud.com/help/en/model-studio/models
  qwen: ["qwen3.7-plus", "qwen3.8-max", "qwen3.8-flash"],
  // Zhipu BigModel
  glm: ["glm-5.3", "glm-5.2", "glm-4.6"],
  // MiniMax Open Platform
  minimax: ["MiniMax-M3", "MiniMax-M2.5", "MiniMax-M2.1"],
  // Moonshot / Kimi
  moonshot: ["kimi-k2.6", "kimi-k2.5", "kimi-latest"],
  // https://developers.openai.com/api/docs/models
  openai: ["gpt-5.6-terra", "gpt-5.6-sol", "gpt-5.6-luna"],
  // https://docs.anthropic.com/en/docs/about-claude/models
  anthropic: ["claude-sonnet-5", "claude-opus-5", "claude-haiku-4-5"],
  // https://ai.google.dev/gemini-api/docs/models
  gemini: ["gemini-3.8-flash", "gemini-3.1-pro-preview", "gemini-3-flash-preview"],
  siliconflow: [
    "deepseek-ai/DeepSeek-V3.2",
    "Qwen/Qwen3-235B-A22B",
    "moonshotai/Kimi-K2-Instruct",
  ],
  openrouter: [
    "anthropic/claude-sonnet-5",
    "google/gemini-3.8-flash",
    "openai/gpt-5.6-terra",
  ],
  groq: [
    "llama-3.3-70b-versatile",
    "meta-llama/llama-4-maverick-17b-128e-instruct",
    "openai/gpt-oss-120b",
  ],
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
  siliconflow: "SiliconFlow",
  openrouter: "OpenRouter",
  groq: "Groq",
};

const PRESET_URL_MATCHERS: Array<{ match: RegExp; id: string }> = [
  { match: /api\.deepseek\.com/i, id: "deepseek" },
  { match: /dashscope/i, id: "qwen" },
  { match: /bigmodel\.cn|api\.z\.ai/i, id: "glm" },
  { match: /minimaxi\.com|minimax\.io/i, id: "minimax" },
  { match: /moonshot\.(cn|ai)/i, id: "moonshot" },
  { match: /api\.openai\.com/i, id: "openai" },
  { match: /api\.anthropic\.com/i, id: "anthropic" },
  { match: /generativelanguage\.googleapis\.com/i, id: "gemini" },
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
