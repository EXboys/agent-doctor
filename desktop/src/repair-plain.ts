import { t } from "./i18n";
import { escapeHtml } from "./format";

export type RepairCheckLike = {
  id?: string;
  title: string;
  status: string;
  message: string;
  details?: string[];
};

export type PlainRepairCheck = {
  title: string;
  message: string;
  /** Original English / paths — shown only under 详情. */
  techDetail: string | null;
};

export function isGatewayConnectivityCheck(check: RepairCheckLike): boolean {
  return (
    check.id === "gateway.connectivity" ||
    check.title === "Gateway connectivity" ||
    /gateway (TCP|DNS|host|URL)/i.test(check.message)
  );
}

function isUpstreamVersionCheck(check: {
  id?: string;
  title?: string;
  message: string;
}): boolean {
  return (
    check.id === "binary.upstream_version" ||
    check.title === "Upstream version" ||
    /matches the latest known release|Upgrading may break settings/i.test(check.message)
  );
}

function parseUpstreamDetail(
  details: string[],
  key: "local" | "latest" | "recommended",
): string | undefined {
  const prefix = `${key}=`;
  return details.find((item) => item.startsWith(prefix))?.slice(prefix.length);
}

export function plainUpstreamVersionCopy(check: {
  status: string;
  message: string;
  details?: string[];
  id?: string;
  title?: string;
}): { title: string; message: string } | null {
  if (!isUpstreamVersionCheck(check)) {
    return null;
  }
  const details = check.details ?? [];
  const local = parseUpstreamDetail(details, "local") || "—";
  const latest = parseUpstreamDetail(details, "latest") || "—";
  const recommended = parseUpstreamDetail(details, "recommended");
  if (check.status === "pass") {
    return {
      title: t("repair.upstreamVersionOkTitle"),
      message: t("repair.upstreamVersionOkDesc", { local }),
    };
  }
  let message = t("repair.upstreamVersionDesc", { local, latest });
  if (recommended && recommended !== latest) {
    message = `${message} ${t("repair.upstreamVersionRecommend", { recommended })}`;
  }
  return {
    title: t("repair.upstreamVersionTitle"),
    message,
  };
}

function isLegacyAgentsCheck(check: RepairCheckLike): boolean {
  return (
    check.id === "openclaw.schema.legacy_agents_list" ||
    check.message.includes("agents.list is a legacy key")
  );
}

function idFamily(id: string): string {
  if (id.startsWith("binary.")) return id;
  if (id.includes("api_key.duplicates")) return "api_key.duplicates";
  if (id.includes("api_key")) return "api_key";
  if (id.includes("permissions")) return "permissions";
  if (id.includes("env.parse") || id.includes("env.read")) return "env.file";
  if (id.startsWith("gateway.")) return id;
  if (id.includes("config.schema")) return "config.schema";
  if (id.startsWith("config.")) return id.split(":")[0] ?? id;
  if (id.includes("mcp.browser") || /\.browser/.test(id)) return "mcp.browser";
  if (id.includes("legacy_agents")) return "legacy_agents";
  if (id.includes("legacy") || id.includes("schema.")) return "schema.legacy";
  if (id.includes("wire_api")) return "codex.wire_api";
  if (id.includes("auth.placeholder")) return "codex.auth.placeholder";
  if (id.includes("mode.overlay")) return "mode.overlay";
  if (id.includes("env.conflicts") || id === "env.conflicts") return "env.conflicts";
  if (id.includes("version.pinned") || id.includes("upstream_version")) return "version";
  if (id.includes("paths.references")) return "paths.references";
  if (id.includes("model_unroutable")) return "model.unroutable";
  if (id.includes("env_stale")) return "env.stale";
  if (id.includes("provider")) return "provider";
  return "";
}

function looksTechnical(text: string): boolean {
  if (!text.trim()) return false;
  if (/[~\/\\]|\.json|\.env|\.ts|\.md|PATH|MCP|CLI|cwd|exit\s*\d/i.test(text)) return true;
  const letters = text.replace(/[^A-Za-z]/g, "");
  return letters.length >= 8 && letters.length >= text.replace(/\s/g, "").length * 0.55;
}

function buildTechDetail(check: RepairCheckLike): string | null {
  const lines: string[] = [];
  const message = check.message.trim();
  const title = check.title.trim();
  if (message) {
    lines.push(message.length < 12 && title ? `${title} · ${message}` : message);
  } else if (title) {
    lines.push(title);
  }
  for (const detail of check.details ?? []) {
    if (detail.trim()) lines.push(detail.trim());
  }
  return lines.length ? lines.join("\n") : null;
}

type FamilyCopy = { okTitle: string; okMsg: string; badTitle: string; badMsg: string };

function familyCopy(family: string): FamilyCopy | null {
  const map: Record<string, FamilyCopy> = {
    "binary.exists": {
      okTitle: t("repair.check.binaryOkTitle"),
      okMsg: t("repair.check.binaryOkDesc"),
      badTitle: t("repair.check.binaryMissingTitle"),
      badMsg: t("repair.check.binaryMissingDesc"),
    },
    "binary.version": {
      okTitle: t("repair.check.versionOkTitle"),
      okMsg: t("repair.check.versionOkDesc"),
      badTitle: t("repair.check.versionBadTitle"),
      badMsg: t("repair.check.versionBadDesc"),
    },
    "binary.path_conflict": {
      okTitle: t("repair.check.pathConflictOkTitle"),
      okMsg: t("repair.check.pathConflictOkDesc"),
      badTitle: t("repair.check.pathConflictTitle"),
      badMsg: t("repair.check.pathConflictDesc"),
    },
    api_key: {
      okTitle: t("repair.check.keyOkTitle"),
      okMsg: t("repair.check.keyOkDesc"),
      badTitle: t("repair.check.keyMissingTitle"),
      badMsg: t("repair.check.keyMissingDesc"),
    },
    "api_key.duplicates": {
      okTitle: t("repair.check.keyOkTitle"),
      okMsg: t("repair.check.keyOkDesc"),
      badTitle: t("repair.check.keyDupTitle"),
      badMsg: t("repair.check.keyDupDesc"),
    },
    permissions: {
      okTitle: t("repair.check.permOkTitle"),
      okMsg: t("repair.check.permOkDesc"),
      badTitle: t("repair.check.permTitle"),
      badMsg: t("repair.check.permDesc"),
    },
    "env.file": {
      okTitle: t("repair.check.envFileOkTitle"),
      okMsg: t("repair.check.envFileOkDesc"),
      badTitle: t("repair.check.envFileTitle"),
      badMsg: t("repair.check.envFileDesc"),
    },
    "gateway.connectivity": {
      okTitle: t("repair.check.gatewayOkTitle"),
      okMsg: t("repair.check.gatewayOkDesc"),
      badTitle: t("repair.gatewayUnreachableTitle"),
      badMsg: t("repair.gatewayUnreachableDesc"),
    },
    "gateway.configured": {
      okTitle: t("repair.check.gatewayCfgOkTitle"),
      okMsg: t("repair.check.gatewayCfgOkDesc"),
      badTitle: t("repair.check.gatewayCfgTitle"),
      badMsg: t("repair.check.gatewayCfgDesc"),
    },
    "gateway.profile_read": {
      okTitle: t("repair.check.gatewayCfgOkTitle"),
      okMsg: t("repair.check.gatewayCfgOkDesc"),
      badTitle: t("repair.check.gatewayCfgTitle"),
      badMsg: t("repair.check.gatewayCfgDesc"),
    },
    "config.schema": {
      okTitle: t("repair.check.schemaOkTitle"),
      okMsg: t("repair.check.schemaOkDesc"),
      badTitle: t("repair.check.schemaTitle"),
      badMsg: t("repair.check.schemaDesc"),
    },
    "config.exists": {
      okTitle: t("repair.check.configOkTitle"),
      okMsg: t("repair.check.configOkDesc"),
      badTitle: t("repair.check.configMissingTitle"),
      badMsg: t("repair.check.configMissingDesc"),
    },
    "config.parse": {
      okTitle: t("repair.check.configOkTitle"),
      okMsg: t("repair.check.configOkDesc"),
      badTitle: t("repair.check.configParseTitle"),
      badMsg: t("repair.check.configParseDesc"),
    },
    "config.read": {
      okTitle: t("repair.check.configOkTitle"),
      okMsg: t("repair.check.configOkDesc"),
      badTitle: t("repair.check.configReadTitle"),
      badMsg: t("repair.check.configReadDesc"),
    },
    "config.paths": {
      okTitle: t("repair.check.configOkTitle"),
      okMsg: t("repair.check.configOkDesc"),
      badTitle: t("repair.check.configMissingTitle"),
      badMsg: t("repair.check.configMissingDesc"),
    },
    "config.optional": {
      okTitle: t("repair.check.configOkTitle"),
      okMsg: t("repair.check.configOkDesc"),
      badTitle: t("repair.check.configMissingTitle"),
      badMsg: t("repair.check.configMissingDesc"),
    },
    "mcp.browser": {
      okTitle: t("repair.check.browserOkTitle"),
      okMsg: t("repair.check.browserOkDesc"),
      badTitle: t("repair.check.browserTitle"),
      badMsg: t("repair.check.browserDesc"),
    },
    "schema.legacy": {
      okTitle: t("repair.check.schemaOkTitle"),
      okMsg: t("repair.check.schemaOkDesc"),
      badTitle: t("repair.check.schemaTitle"),
      badMsg: t("repair.check.schemaDesc"),
    },
    legacy_agents: {
      okTitle: t("repair.check.schemaOkTitle"),
      okMsg: t("repair.check.schemaOkDesc"),
      badTitle: t("repair.openclawLegacyAgentsTitle"),
      badMsg: t("repair.openclawLegacyAgentsDesc"),
    },
    "codex.wire_api": {
      okTitle: t("repair.check.schemaOkTitle"),
      okMsg: t("repair.check.schemaOkDesc"),
      badTitle: t("repair.check.codexWireTitle"),
      badMsg: t("repair.check.codexWireDesc"),
    },
    "codex.auth.placeholder": {
      okTitle: t("repair.check.keyOkTitle"),
      okMsg: t("repair.check.keyOkDesc"),
      badTitle: t("repair.check.codexAuthTitle"),
      badMsg: t("repair.check.codexAuthDesc"),
    },
    "mode.overlay": {
      okTitle: t("repair.check.modeOkTitle"),
      okMsg: t("repair.check.modeOkDesc"),
      badTitle: t("repair.check.modeTitle"),
      badMsg: t("repair.check.modeDesc"),
    },
    "env.conflicts": {
      okTitle: t("repair.check.envOkTitle"),
      okMsg: t("repair.check.envOkDesc"),
      badTitle: t("repair.check.envTitle"),
      badMsg: t("repair.check.envDesc"),
    },
    "paths.references": {
      okTitle: t("repair.check.pathsOkTitle"),
      okMsg: t("repair.check.pathsOkDesc"),
      badTitle: t("repair.check.pathsTitle"),
      badMsg: t("repair.check.pathsDesc"),
    },
    "model.unroutable": {
      okTitle: t("repair.check.modelOkTitle"),
      okMsg: t("repair.check.modelOkDesc"),
      badTitle: t("repair.check.modelTitle"),
      badMsg: t("repair.check.modelDesc"),
    },
    "env.stale": {
      okTitle: t("repair.check.keyOkTitle"),
      okMsg: t("repair.check.keyOkDesc"),
      badTitle: t("repair.check.keyStaleTitle"),
      badMsg: t("repair.check.keyStaleDesc"),
    },
    provider: {
      okTitle: t("repair.check.providerOkTitle"),
      okMsg: t("repair.check.providerOkDesc"),
      badTitle: t("repair.check.providerTitle"),
      badMsg: t("repair.check.providerDesc"),
    },
    version: {
      okTitle: t("repair.check.versionOkTitle"),
      okMsg: t("repair.check.versionOkDesc"),
      badTitle: t("repair.check.versionPinTitle"),
      badMsg: t("repair.check.versionPinDesc"),
    },
  };
  return map[family] ?? null;
}

function inferFamilyFromTitle(check: RepairCheckLike): string {
  const blob = `${check.title} ${check.message}`;
  if (/Gateway connectivity/i.test(blob)) return "gateway.connectivity";
  if (/API key/i.test(blob)) return "api_key";
  if (/permission/i.test(blob)) return "permissions";
  if (/Binary exists|not installed/i.test(blob)) return "binary.exists";
  if (/Multiple installs/i.test(blob)) return "binary.path_conflict";
  if (/Upstream version/i.test(blob)) return "binary.upstream_version";
  if (/Version command/i.test(blob)) return "binary.version";
  if (/Config schema|schema/i.test(blob)) return "config.schema";
  if (/Config (exists|parse|read)/i.test(blob)) return "config.parse";
  if (/MCP|browser|Skills path/i.test(blob)) return "mcp.browser";
  if (/wire_api/i.test(blob)) return "codex.wire_api";
  if (/placeholder auth/i.test(blob)) return "codex.auth.placeholder";
  if (/Environment variables/i.test(blob)) return "env.conflicts";
  if (/Gateway configured|Gateway profile/i.test(blob)) return "gateway.configured";
  if (/overlay|Mode /i.test(blob)) return "mode.overlay";
  if (/path reference/i.test(blob)) return "paths.references";
  return "";
}

/**
 * Beginner-facing copy for a probe/repair check.
 * English titles, paths, and raw messages go into techDetail.
 */
export function plainRepairCheckCopy(check: RepairCheckLike): PlainRepairCheck {
  const techDetail = buildTechDetail(check);
  const ok = check.status === "pass" || check.status === "n/a" || check.status === "not checked";

  if (isGatewayConnectivityCheck(check)) {
    return {
      title: ok ? t("repair.check.gatewayOkTitle") : t("repair.gatewayUnreachableTitle"),
      message: ok ? t("repair.check.gatewayOkDesc") : t("repair.gatewayUnreachableDesc"),
      techDetail,
    };
  }

  if (isLegacyAgentsCheck(check)) {
    return {
      title: ok ? t("repair.check.schemaOkTitle") : t("repair.openclawLegacyAgentsTitle"),
      message: ok ? t("repair.check.schemaOkDesc") : t("repair.openclawLegacyAgentsDesc"),
      techDetail,
    };
  }

  const upstream = plainUpstreamVersionCopy(check);
  if (upstream) {
    return { title: upstream.title, message: upstream.message, techDetail };
  }

  const id = (check.id ?? "").trim();
  const family = (id ? idFamily(id) : "") || inferFamilyFromTitle(check);
  const copy = family ? familyCopy(family) : null;
  if (copy) {
    return {
      title: ok ? copy.okTitle : copy.badTitle,
      message: ok ? copy.okMsg : copy.badMsg,
      techDetail,
    };
  }

  if (ok) {
    return {
      title: t("repair.check.genericOkTitle"),
      message: looksTechnical(check.message)
        ? t("repair.check.genericOkDesc")
        : check.message || t("repair.check.genericOkDesc"),
      techDetail,
    };
  }

  return {
    title: t("repair.check.genericBadTitle"),
    message: t("repair.check.genericBadDesc"),
    techDetail,
  };
}

export function renderRepairCheckTechDetails(techDetail: string | null): string {
  if (!techDetail) return "";
  return `<span class="repair-check-tech" title="${escapeHtml(techDetail)}">${escapeHtml(techDetail)}</span>`;
}

export function renderPlainRepairCheckBody(check: RepairCheckLike): string {
  const plain = plainRepairCheckCopy(check);
  const tech = renderRepairCheckTechDetails(plain.techDetail);
  return `<span class="repair-check-body${tech ? " has-tech" : ""}">
    <span class="repair-check-main">
      <strong>${escapeHtml(plain.title)}</strong>
      <span>${escapeHtml(plain.message)}</span>
    </span>
    ${tech}
  </span>`;
}
