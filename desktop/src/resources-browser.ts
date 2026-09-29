
import { t } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import type { BrowserMcpTargetAction, BrowserMcpTargetStatus, McpInventoryItem, McpModuleStatus, ResourceRow } from "./types";
import { resourcesState, MCP_SHOW_UI_KEY, MCP_USER_DATA_DIR_KEY, MCP_PROFILE_DIRECTORY_KEY, mcpBrowserBadgeEl, mcpChromeEl, mcpCdpEl, mcpConfiguredEl, mcpBinaryEl, mcpShowUiEl, mcpUserDataDirEl, mcpProfileDirectoryEl, mcpProfileSystemEl, mcpProfileIsolatedEl, mcpDiagnoseWireEl, mcpTargetsEl, mcpSnippetEl, mcpFootnoteEl } from "./resources-state";
import { renderResourcesList } from "./resources";
import { mcpStatus, mcpDiagnoseWire } from "./ipc";

export function persistShowBrowserUi(show: boolean): void {
  try {
    localStorage.setItem(MCP_SHOW_UI_KEY, show ? "1" : "0");
  } catch {
    // ignore
  }
}

export function persistUserDataDir(path: string): void {
  try {
    localStorage.setItem(MCP_USER_DATA_DIR_KEY, path);
  } catch {
    // ignore
  }
}

export function persistProfileDirectory(name: string): void {
  try {
    localStorage.setItem(MCP_PROFILE_DIRECTORY_KEY, name.trim() || "Default");
  } catch {
    // ignore
  }
}

export function isShowBrowserUi(): boolean {
  return mcpShowUiEl.checked;
}

export function selectedUserDataDir(): string {
  return mcpUserDataDirEl.value.trim();
}

export function selectedProfileDirectory(): string {
  return mcpProfileDirectoryEl.value.trim() || "Default";
}

export function configuredBrowserArg(status: McpModuleStatus, flag: string): string | null {
  const browser = status.inventory.servers.find((server) => server.is_browser);
  if (!browser) return null;
  const idx = browser.args.findIndex((arg) => arg === flag);
  if (idx >= 0 && browser.args[idx + 1]) return browser.args[idx + 1];
  return null;
}

export function syncShowBrowserUiPreference(status: McpModuleStatus): void {
  try {
    const saved = localStorage.getItem(MCP_SHOW_UI_KEY);
    if (saved === "0" || saved === "1") {
      mcpShowUiEl.checked = saved === "1";
      return;
    }
  } catch {
    // fall through
  }
  const browser = status.inventory.servers.find((server) => server.is_browser);
  mcpShowUiEl.checked = browser ? !browser.args.includes("--headless") : true;
}

export function syncProfileModeButtons(): void {
  const dir = selectedUserDataDir();
  const isolated = resourcesState.lastMcpStatus?.browser.isolated_user_data_dir || "";
  const system = resourcesState.lastMcpStatus?.browser.system_user_data_dir || "";
  mcpProfileIsolatedEl.classList.toggle("is-active", Boolean(isolated) && dir === isolated);
  mcpProfileSystemEl.classList.toggle("is-active", Boolean(system) && dir === system);
}

export function syncUserDataDirPreference(status: McpModuleStatus): void {
  const chrome = status.browser;
  const fromConfig = configuredBrowserArg(status, "--user-data-dir");
  let saved: string | null = null;
  try {
    saved = localStorage.getItem(MCP_USER_DATA_DIR_KEY);
  } catch {
    saved = null;
  }
  mcpUserDataDirEl.value =
    (fromConfig && fromConfig.trim()) ||
    (saved && saved.trim()) ||
    chrome.isolated_user_data_dir ||
    chrome.user_data_dir ||
    "";

  const profileFromConfig = configuredBrowserArg(status, "--profile-directory");
  let savedProfile: string | null = null;
  try {
    savedProfile = localStorage.getItem(MCP_PROFILE_DIRECTORY_KEY);
  } catch {
    savedProfile = null;
  }
  mcpProfileDirectoryEl.value =
    (profileFromConfig && profileFromConfig.trim()) ||
    (savedProfile && savedProfile.trim()) ||
    chrome.profile_directory ||
    "Default";
  syncProfileModeButtons();
}

export function refreshMcpSnippet(): void {
  if (!resourcesState.lastMcpStatus) return;
  const port = resourcesState.lastMcpStatus.browser.port;
  const args = ["mcp", "browser", "--port", String(port)];
  if (!isShowBrowserUi()) args.push("--headless");
  const dir = selectedUserDataDir();
  if (dir) args.push("--user-data-dir", dir);
  args.push("--profile-directory", selectedProfileDirectory());
  mcpSnippetEl.textContent = JSON.stringify(
    {
      mcpServers: {
        browser: {
          command: resourcesState.lastMcpStatus.binary,
          args,
        },
      },
    },
    null,
    2,
  );
}

export function targetStatusLabel(target: BrowserMcpTargetStatus): string {
  if (!target.installed) return t("mcp.targetNotInstalled");
  return target.configured
    ? t("mcp.targetInstalledConfigured")
    : t("mcp.targetInstalledMissing");
}

export function targetActionLabel(action: BrowserMcpTargetAction): string {
  if (action.action === "wrote") return t("mcp.targetWrote");
  if (action.action === "skipped_not_installed") return t("mcp.targetSkipped");
  return t("mcp.targetFailed");
}

export function renderMcpTargets(
  targets: BrowserMcpTargetStatus[],
  actions: BrowserMcpTargetAction[] | null = resourcesState.lastWireActions,
): void {
  mcpTargetsEl.replaceChildren();
  for (const target of targets) {
    const action = actions?.find((item) => item.runtime_id === target.runtime_id);
    const li = document.createElement("li");
    li.className = "mcp-target-row";
    const installed = action?.installed ?? target.installed;
    const configured = action?.action === "wrote" ? true : target.configured;
    const tone = action
      ? action.ok
        ? action.action === "wrote"
          ? "ok"
          : "muted"
        : "bad"
      : !installed
        ? "muted"
        : configured
          ? "ok"
          : "warn";
    li.classList.add(`tone-${tone}`);

    const name = document.createElement("strong");
    name.textContent = target.display_name;
    const status = document.createElement("span");
    status.className = "mcp-target-status";
    status.textContent = action ? targetActionLabel(action) : targetStatusLabel(target);
    const detail = document.createElement("span");
    detail.className = "mcp-target-detail";
    detail.textContent =
      action?.message || (installed ? target.runtime_id : t("mcp.targetNotInstalled"));
    li.append(name, status, detail);
    mcpTargetsEl.appendChild(li);
  }
}

export function renderMcpBrowserStatus(status: McpModuleStatus): void {
  resourcesState.lastMcpStatus = status;
  const chrome = status.browser;
  mcpChromeEl.textContent = chrome.chrome_found
    ? t("mcp.chromeOk", { version: chrome.version || chrome.binary || "OK" })
    : t("mcp.chromeMissing");
  mcpChromeEl.title = chrome.binary || "";
  mcpCdpEl.textContent = chrome.cdp_connected
    ? t("mcp.cdpConnected")
    : t("mcp.cdpIdle");
  mcpCdpEl.title = chrome.port ? `:${chrome.port}` : "";
  mcpConfiguredEl.textContent =
    status.configured_runtimes.length > 0
      ? t("mcp.configuredList", { list: status.configured_runtimes.join(", ") })
      : t("mcp.configuredNone");
  mcpBinaryEl.textContent = status.binary;
  mcpBinaryEl.title = status.binary;
  syncShowBrowserUiPreference(status);
  syncUserDataDirPreference(status);
  refreshMcpSnippet();
  renderMcpTargets(status.targets ?? []);

  const snippetError =
    status.config_snippet &&
    typeof status.config_snippet === "object" &&
    status.config_snippet !== null &&
    "error" in status.config_snippet
      ? String((status.config_snippet as { error?: unknown }).error ?? "")
      : "";
  const cliBroken =
    !status.binary.trim() ||
    /VCRUNTIME|Visual C\+\+|could not start|Could not find the Agent Doctor CLI/i.test(snippetError);
  if (!resourcesState.mcpConfigureInFlight) {
    mcpFootnoteEl.textContent = cliBroken ? t("mcp.cliUnresolved") : "";
  }

  mcpBrowserBadgeEl.classList.remove("ok", "warn", "muted", "bad");
  if (cliBroken) {
    mcpBrowserBadgeEl.textContent = t("mcp.badgePartial");
    mcpBrowserBadgeEl.classList.add("warn");
  } else if (!chrome.chrome_found) {
    mcpBrowserBadgeEl.textContent = t("mcp.badgeMissing");
    mcpBrowserBadgeEl.classList.add("bad");
  } else if (status.configured_runtimes.length > 0) {
    mcpBrowserBadgeEl.textContent = t("mcp.badgeReady");
    mcpBrowserBadgeEl.classList.add("ok");
  } else {
    mcpBrowserBadgeEl.textContent = t("mcp.badgePartial");
    mcpBrowserBadgeEl.classList.add("warn");
  }

  const canWire = chrome.chrome_found && !cliBroken && !resourcesState.mcpConfigureInFlight;
  mcpDiagnoseWireEl.disabled = !canWire;
}

export function browserToolRow(): ResourceRow | null {
  if (!resourcesState.lastMcpStatus) return null;
  const ready =
    resourcesState.lastMcpStatus.browser.chrome_found && resourcesState.lastMcpStatus.configured_runtimes.length > 0;
  const agents = resourcesState.lastMcpStatus.configured_runtimes
    .map((runtime) => {
      if (runtime === "claude-code") return "Claude";
      if (runtime === "codex") return "Codex";
      if (runtime === "openclaw") return "OpenClaw";
      if (runtime === "hermes") return "Hermes";
      if (runtime === "deepseek-harness") return "DeepSeek";
      return runtime;
    })
    .filter(Boolean);
  return {
    kind: "mcp",
    name: t("resources.toolBrowserName"),
    sub: agents.length
      ? `${t("resources.toolBrowserDesc")} ${agents.join(" · ")}`
      : t("resources.toolBrowserDesc"),
    meta: ready ? t("resources.toolBrowserReady") : t("resources.toolBrowserNeedSetup"),
    tone: ready ? "ok" : "warn",
    issue: !ready,
    action: "open-browser",
  };
}

export function buildMcpRows(): ResourceRow[] {
  const rows: ResourceRow[] = [];
  const browser = browserToolRow();
  if (browser) rows.push(browser);
  const mcpGroups = new Map<string, McpInventoryItem[]>();
  for (const server of resourcesState.lastMcpStatus?.inventory.servers ?? []) {
    if (server.is_browser) continue; // Browser has its own tab.
    const key = server.name.trim().toLowerCase();
    const group = mcpGroups.get(key) ?? [];
    group.push(server);
    mcpGroups.set(key, group);
  }

  for (const servers of mcpGroups.values()) {
    const primary = servers[0]!;
    const runtimes = [...new Set(servers.map((server) => server.runtime_hint))].map((runtime) => {
      if (runtime === "claude-code") return "Claude";
      if (runtime === "codex") return "Codex";
      if (runtime === "openclaw") return "OpenClaw";
      if (runtime === "hermes") return "Hermes";
      if (runtime === "deepseek-harness") return "DeepSeek";
      if (runtime === "shared") return t("resources.shared");
      return runtime;
    });
    const issues = servers.filter((server) => !server.healthy);
    const bindingLabel = t("resources.mcpBindings", { count: String(servers.length) });
    rows.push({
      kind: "mcp",
      name: primary.name,
      sub: runtimes.join(" · "),
      meta:
        issues.length > 0
          ? t("resources.mcpBindingIssues", {
              issues: String(issues.length),
              count: String(servers.length),
            })
          : `${t("resources.mcpHealthy")} · ${bindingLabel}`,
      tone: issues.length > 0 ? "bad" : "ok",
      issue: issues.length > 0,
    });
  }
  return rows.sort((a, b) => a.name.localeCompare(b.name));
}

export async function loadMcpStatus(): Promise<void> {
  try {
    const status = await mcpStatus({
      port: null,
      probeChrome: false,
      discoverChrome: resourcesState.activeSection === "browser" || resourcesState.activeSection === "tools",
    });
    renderMcpBrowserStatus(status);
    renderResourcesList();
  } catch (error) {
    resourcesState.lastMcpStatus = null;
    mcpBrowserBadgeEl.textContent = "—";
    mcpBrowserBadgeEl.className = "badge muted";
    mcpFootnoteEl.textContent = withErrorDetail(t("mcp.loadFailed"), error);
    mcpTargetsEl.replaceChildren();
    renderResourcesList();
  }
}

export async function diagnoseAndWireBrowserMcp(): Promise<void> {
  if (resourcesState.mcpConfigureInFlight) return;
  resourcesState.mcpConfigureInFlight = true;
  mcpDiagnoseWireEl.disabled = true;
  mcpFootnoteEl.textContent = t("mcp.configuring");
  try {
    const showUi = isShowBrowserUi();
    persistShowBrowserUi(showUi);
    const userDataDir = selectedUserDataDir();
    const profileDirectory = selectedProfileDirectory();
    persistUserDataDir(userDataDir);
    persistProfileDirectory(profileDirectory);
    const report = await mcpDiagnoseWire({
      port: null,
      headless: !showUi,
      userDataDir: userDataDir || null,
      profileDirectory,
    });
    resourcesState.lastWireActions = report.targets;
    if (report.wrote > 0) {
      mcpFootnoteEl.textContent = t("mcp.diagnoseWireOk", {
        wrote: String(report.wrote),
        skipped: String(report.skipped),
        failed: String(report.failed),
      });
    } else if (report.issues.length > 0) {
      mcpFootnoteEl.textContent = report.issues[0]?.message || t("mcp.diagnoseWireNone");
    } else {
      mcpFootnoteEl.textContent = t("mcp.diagnoseWireNone");
    }
    await loadMcpStatus();
    if (resourcesState.lastMcpStatus) {
      renderMcpTargets(resourcesState.lastMcpStatus.targets ?? [], resourcesState.lastWireActions);
    }
  } catch (error) {
    mcpFootnoteEl.textContent = withErrorDetail(t("mcp.configureFailed"), error);
  } finally {
    resourcesState.mcpConfigureInFlight = false;
    const chromeOk = resourcesState.lastMcpStatus?.browser.chrome_found ?? false;
    const cliOk = Boolean(resourcesState.lastMcpStatus?.binary?.trim());
    mcpDiagnoseWireEl.disabled = !(chromeOk && cliOk);
  }
}
