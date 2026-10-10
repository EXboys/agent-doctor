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
    url: "https://api.minimax.cn/v1",
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
  volcengine: {
    name: "火山方舟",
    url: "https://ark.cn-beijing.volces.com/api/v3",
    protocol: "openai",
    models: modelsForPresetId("volcengine"),
    chip: "火山方舟",
  },
  qianfan: {
    name: "千帆",
    url: "https://qianfan.baidubce.com/v2",
    protocol: "openai",
    models: modelsForPresetId("qianfan"),
    chip: "千帆",
  },
  siliconflow: {
    name: "SiliconFlow",
    url: "https://api.siliconflow.cn/v1",
    protocol: "openai",
    models: modelsForPresetId("siliconflow"),
    chip: "硅基流动",
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
  { ids: ["volcengine", "qianfan", "siliconflow", "openrouter", "groq"] },
];

export type GlmKeyPlan = "payg" | "coding";

const KEY_PLAN_PRESETS = new Set(["glm", "qwen", "moonshot", "volcengine"]);

export function presetHasKeyPlan(presetId: string): boolean {
  return KEY_PLAN_PRESETS.has(presetId);
}

/** Which saved service a coding-plan or pay-as-you-go address belongs to. */
export function presetIdForPlanUrl(
  url: string,
): "glm" | "qwen" | "moonshot" | "volcengine" | null {
  const lower = url.trim().toLowerCase();
  if (lower.includes("bigmodel.cn") || lower.includes("api.z.ai")) return "glm";
  if (lower.includes("volces.com")) return "volcengine";
  if (lower.includes("dashscope")) return "qwen";
  if (
    lower.includes("api.kimi.com") ||
    lower.includes("api.kimi.ai") ||
    lower.includes("moonshot.cn") ||
    lower.includes("moonshot.ai")
  ) {
    return "moonshot";
  }
  return null;
}

/** Pay-as-you-go and a coding subscription are different keys when this returns a plan. */
export function keyPlanFromUrl(url: string): GlmKeyPlan | null {
  const lower = url.trim().toLowerCase();
  const preset = presetIdForPlanUrl(lower);
  if (!preset) return null;
  if (preset === "glm") return lower.includes("/api/coding/") ? "coding" : "payg";
  if (preset === "volcengine") return lower.includes("/api/coding") ? "coding" : "payg";
  if (preset === "qwen") {
    return lower.includes("coding.dashscope") || lower.includes("coding-intl.dashscope")
      ? "coding"
      : "payg";
  }
  return lower.includes("/coding") ? "coding" : "payg";
}

export function glmKeyPlanFromUrl(url: string): GlmKeyPlan | null {
  return presetIdForPlanUrl(url) === "glm" ? keyPlanFromUrl(url) : null;
}

export function urlForKeyPlan(presetId: string, plan: GlmKeyPlan, currentUrl = ""): string {
  const lower = currentUrl.toLowerCase();
  if (presetId === "volcengine") {
    return plan === "coding"
      ? "https://ark.cn-beijing.volces.com/api/coding/v3"
      : "https://ark.cn-beijing.volces.com/api/v3";
  }
  if (presetId === "qwen") {
    const intl = lower.includes("dashscope-intl") || lower.includes("coding-intl") || lower.includes("dashscope-us");
    if (plan === "coding") {
      return intl
        ? "https://coding-intl.dashscope.aliyuncs.com/v1"
        : "https://coding.dashscope.aliyuncs.com/v1";
    }
    return intl
      ? "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
      : "https://dashscope.aliyuncs.com/compatible-mode/v1";
  }
  if (presetId === "moonshot") {
    const intl = lower.includes("moonshot.ai") || lower.includes("kimi.ai");
    if (plan === "coding") {
      return intl ? "https://api.kimi.ai/coding/v1" : "https://api.kimi.com/coding/v1";
    }
    return intl ? "https://api.moonshot.ai/v1" : "https://api.moonshot.cn/v1";
  }
  const host = lower.includes("api.z.ai") ? "https://api.z.ai" : "https://open.bigmodel.cn";
  return plan === "coding" ? `${host}/api/coding/paas/v4` : `${host}/api/paas/v4`;
}

export function glmUrlForPlan(plan: GlmKeyPlan, currentUrl = ""): string {
  return urlForKeyPlan("glm", plan, currentUrl);
}

export type MinimaxRegion = "cn" | "intl";

export function minimaxRegionFromUrl(url: string): MinimaxRegion | null {
  const lower = url.trim().toLowerCase();
  // api.minimaxi.com is the older China platform host, not the international one.
  if (lower.includes("api.minimax.cn") || lower.includes("api.minimaxi.com")) return "cn";
  if (lower.includes("api.minimax.io")) return "intl";
  return null;
}

export function minimaxUrlForRegion(region: MinimaxRegion): string {
  return region === "cn" ? "https://api.minimax.cn/v1" : "https://api.minimax.io/v1";
}

export function matchProviderPresetId(
  name: string,
  url: string,
  protocol?: string,
): string {
  const normalizedUrl = url.trim().replace(/\/+$/, "");
  if ((!protocol || protocol === "openai") && presetIdForPlanUrl(normalizedUrl)) {
    return presetIdForPlanUrl(normalizedUrl)!;
  }
  if ((!protocol || protocol === "openai") && minimaxRegionFromUrl(normalizedUrl)) {
    return "minimax";
  }
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
