import { modelsForPresetId } from "./provider-models";
import type { ProviderProtocol } from "./types";

export type ProviderPreset = {
  name: string;
  url: string;
  protocol: ProviderProtocol;
  models: string[];
  chip?: string;
};

/** Shared presets for wiring tab and diagnose config step. */
export const PROVIDER_PRESETS: Record<string, ProviderPreset> = {
  deepseek: {
    name: "DeepSeek",
    url: "https://api.deepseek.com/v1",
    protocol: "openai",
    models: modelsForPresetId("deepseek"),
    chip: "DeepSeek",
  },
  qwen: {
    name: "Qwen",
    url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    protocol: "openai",
    models: modelsForPresetId("qwen"),
    chip: "Qwen",
  },
  glm: {
    name: "GLM",
    url: "https://open.bigmodel.cn/api/paas/v4",
    protocol: "openai",
    models: modelsForPresetId("glm"),
    chip: "GLM",
  },
  minimax: {
    name: "MiniMax",
    url: "https://api.minimaxi.com/v1",
    protocol: "openai",
    models: modelsForPresetId("minimax"),
    chip: "MiniMax",
  },
  moonshot: {
    name: "Moonshot / Kimi",
    url: "https://api.moonshot.cn/v1",
    protocol: "openai",
    models: modelsForPresetId("moonshot"),
    chip: "Kimi",
  },
  openai: {
    name: "ChatGPT / OpenAI",
    url: "https://api.openai.com/v1",
    protocol: "openai",
    models: modelsForPresetId("openai"),
    chip: "ChatGPT",
  },
  anthropic: {
    name: "Claude",
    url: "https://api.anthropic.com",
    protocol: "anthropic",
    models: modelsForPresetId("anthropic"),
    chip: "Claude",
  },
  gemini: {
    name: "Gemini",
    url: "https://generativelanguage.googleapis.com/v1beta/openai/",
    protocol: "openai",
    models: modelsForPresetId("gemini"),
    chip: "Gemini",
  },
  siliconflow: {
    name: "SiliconFlow",
    url: "https://api.siliconflow.cn/v1",
    protocol: "openai",
    models: modelsForPresetId("siliconflow"),
    chip: "SiliconFlow",
  },
  openrouter: {
    name: "OpenRouter",
    url: "https://openrouter.ai/api/v1",
    protocol: "openai",
    models: modelsForPresetId("openrouter"),
    chip: "OpenRouter",
  },
  groq: {
    name: "Groq",
    url: "https://api.groq.com/openai/v1",
    protocol: "openai",
    models: modelsForPresetId("groq"),
    chip: "Groq",
  },
};

export const PRESET_PICKER_GROUPS: Array<{ ids: string[] }> = [
  { ids: ["deepseek", "qwen", "glm", "minimax", "moonshot"] },
  { ids: ["openai", "anthropic", "gemini"] },
  { ids: ["siliconflow", "openrouter", "groq"] },
];

export function matchProviderPresetId(
  name: string,
  url: string,
  protocol?: string,
): string {
  const normalizedUrl = url.trim().replace(/\/+$/, "");
  for (const [id, preset] of Object.entries(PROVIDER_PRESETS)) {
    const presetUrl = preset.url.replace(/\/+$/, "");
    if (protocol && preset.protocol !== protocol) {
      continue;
    }
    if (
      normalizedUrl === presetUrl ||
      name.trim().toLowerCase() === preset.name.toLowerCase() ||
      name.trim().toLowerCase() === (preset.chip ?? "").toLowerCase()
    ) {
      return id;
    }
  }
  return "custom";
}
