import { t } from "./i18n";
import type { ClusterModelTier } from "./types";

const GIB = 1024 * 1024 * 1024;

export function llamaModelCopy(tier: ClusterModelTier): { name: string; hint: string } {
  switch (tier.id) {
    case "qwen25-1.5b":
      return { name: t("cluster.m15"), hint: t("cluster.m15Hint") };
    case "qwen25-3b":
      return { name: t("cluster.m3"), hint: t("cluster.m3Hint") };
    case "qwen25-7b":
      return { name: t("cluster.m7"), hint: t("cluster.m7Hint") };
    case "qwen25-coder-7b":
      return { name: t("cluster.mCoder"), hint: t("cluster.mCoderHint") };
    case "deepseek-r1-7b":
      return { name: t("cluster.mR1"), hint: t("cluster.mR1Hint") };
    case "qwen3-8b":
      return { name: t("cluster.mQ3"), hint: t("cluster.mQ3Hint") };
    case "qwen25-14b":
      return { name: t("cluster.m14"), hint: t("cluster.m14Hint") };
    case "qwen3-14b":
      return { name: t("cluster.mQ314"), hint: t("cluster.mQ314Hint") };
    case "qwen3-30b":
      return { name: t("cluster.m30"), hint: t("cluster.m30Hint") };
    case "qwen25-32b":
      return { name: t("cluster.m32"), hint: t("cluster.m32Hint") };
    case "qwen25-72b":
      return { name: t("cluster.m72"), hint: t("cluster.m72Hint") };
    default:
      return { name: tier.id, hint: "" };
  }
}

export function llamaFileSize(bytes: number | undefined): string {
  if (!bytes) return "";
  const gb = bytes / GIB;
  return gb < 10 ? `${gb.toFixed(1)} GB` : `${Math.round(gb)} GB`;
}
