/**
 * Model suggestions shared by home-page wiring (module 2) and Ask composer.
 * Keep ids / URL matchers aligned with PROVIDER_PRESETS in wiring-ui.ts.
 */

export const PRESET_MODELS_BY_ID: Record<string, string[]> = {
  deepseek: ["deepseek-v4-flash", "deepseek-v4-pro"],
  qwen: ["qwen-plus", "qwen-max", "qwen-flash"],
  glm: ["glm-5.3", "glm-5.3-flash", "glm-4.6"],
  minimax: ["MiniMax-M3", "MiniMax-M2.7", "MiniMax-M2.5"],
  moonshot: ["kimi-k2.5", "kimi-latest", "moonshot-v1-auto"],
  openai: ["gpt-4.1-mini", "gpt-4.1", "o4-mini"],
  anthropic: ["claude-sonnet-4-5", "claude-opus-4-5", "claude-haiku-4-5"],
  gemini: ["gemini-2.5-flash", "gemini-2.5-pro", "gemini-3-flash-preview"],
  siliconflow: ["deepseek-ai/DeepSeek-V3.2", "Qwen/Qwen3-235B-A22B"],
  openrouter: ["openai/gpt-4.1-mini", "google/gemini-2.5-flash", "anthropic/claude-sonnet-4.5"],
  groq: ["llama-3.3-70b-versatile", "openai/gpt-oss-120b"],
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

function presetIdForUrl(url: string): string | null {
  return PRESET_URL_MATCHERS.find((row) => row.match.test(url))?.id ?? null;
}

/** Build selectable model list for a wired provider URL + current model. */
export function modelsForProviderUrl(url: string, currentModel?: string | null): string[] {
  const id = presetIdForUrl(url);
  const list = id ? modelsForPresetId(id) : [];
  const current = currentModel?.trim();
  if (current && !list.includes(current)) {
    list.unshift(current);
  }
  return list;
}

/** Compact provider label for Ask composer (chip when known, else stored name). */
export function providerChipForUrl(url: string, fallbackName: string): string {
  const id = presetIdForUrl(url);
  if (id && PRESET_CHIPS_BY_ID[id]) return PRESET_CHIPS_BY_ID[id];
  const name = fallbackName.trim();
  return name || "Provider";
}
