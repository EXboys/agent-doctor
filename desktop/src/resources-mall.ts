
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "./i18n";
import { teamupsLoginFailure, teamupsMallInstallFailure, withErrorDetail } from "./friendly-error";
import type { TeamupsAccountStatus, TeamupsCatalogItem } from "./types";
import { resourcesState, toolFiltersEl, toolsListEl, toolsEmptyEl, toolsFootnoteEl, mallAccountEl, mallAccountStatusEl, mallLoginEl, mallLogoutEl } from "./resources-state";
import type { MallFilter } from "./resources-state";
import { defaultToolsFootnote, renderResourcesList, rowMatchesQuery, setSkillsFootnote, skillsPanelActive, syncSkillsFootnote } from "./resources";
import { buildMcpRows, loadMcpStatus } from "./resources-browser";
import { appendFilterChip, appendResourceRow, appendUnifiedSkillRow, entryFromMallItem, isStoreScope, loadSkills, renderSkillsList } from "./resources-skills";
import { listTeamupsMallCatalog, teamupsAccountStatus, pollTeamupsLogin, startTeamupsLogin, signOutTeamups, installTeamupsMallItem } from "./ipc";

export function applyAccountOwnership(): void {
  if (!resourcesState.lastMallCatalog || !resourcesState.lastTeamupsAccount?.signed_in) return;
  const account = resourcesState.lastTeamupsAccount as TeamupsAccountStatus & {
    packCount?: number;
  };
  const owned = new Set(
    (account.packs ?? []).map((slug) => slug.trim()).filter(Boolean),
  );
  if (owned.size === 0) return;
  for (const item of resourcesState.lastMallCatalog.items) {
    if (owned.has(item.id) || (item.pack_slug && owned.has(item.pack_slug))) {
      item.owned = true;
    }
  }
  const base = resourcesState.lastMallCatalog.base_url.replace(/\/$/, "");
  for (const slug of owned) {
    if (resourcesState.lastMallCatalog.items.some((item) => item.id === slug)) continue;
    resourcesState.lastMallCatalog.items.push({
      id: slug,
      kind: "pack",
      name: slug,
      description: "",
      free: false,
      price_label: null,
      owned: true,
      installed: false,
      skill_count: null,
      pack_slug: slug,
      purchase_url: base ? `${base}/packs/${slug}` : null,
      version: null,
    });
  }
}

export function isMallToolItem(item: TeamupsCatalogItem): boolean {
  const kind = item.kind.trim().toLowerCase();
  return kind === "mcp" || kind === "tool" || kind === "tools";
}

export function mallToolPool(): TeamupsCatalogItem[] {
  return (resourcesState.lastMallCatalog?.items ?? []).filter(isMallToolItem);
}

export function filteredMallToolItems(): TeamupsCatalogItem[] {
  return mallToolPool()
    .filter((item) => {
      if (resourcesState.mallFilter === "free" && !item.free) return false;
      if (resourcesState.mallFilter === "paid" && item.free) return false;
      if (!resourcesState.resourceQuery) return true;
      const blob = `${item.name} ${item.description} ${item.id}`.toLowerCase();
      return blob.includes(resourcesState.resourceQuery);
    })
    .sort((a, b) => Number(b.owned) - Number(a.owned));
}

export function filteredMallItems(): TeamupsCatalogItem[] {
  const items = resourcesState.lastMallCatalog?.items ?? [];
  return items
    .filter((item) => {
      if (resourcesState.mallFilter === "free" && !item.free) return false;
      if (resourcesState.mallFilter === "paid" && item.free) return false;
      if (resourcesState.mallFilter === "pack" && item.kind !== "pack") return false;
      if (resourcesState.mallFilter === "skill" && item.kind !== "skill") return false;
      if (!resourcesState.resourceQuery) return true;
      const blob = `${item.name} ${item.description} ${item.id} ${item.kind}`.toLowerCase();
      return blob.includes(resourcesState.resourceQuery);
    })
    .sort((a, b) => Number(b.owned) - Number(a.owned));
}

export function renderToolsList(): void {
  if (isStoreScope()) {
    if (resourcesState.mallFilter === "pack" || resourcesState.mallFilter === "skill") resourcesState.mallFilter = "all";
    toolFiltersEl.replaceChildren();
    const mallChips: Array<{ id: MallFilter; label: string }> = [
      { id: "all", label: t("resources.filterAll") },
      { id: "free", label: t("resources.mallFilterFree") },
      { id: "paid", label: t("resources.mallFilterPaid") },
    ];
    for (const chip of mallChips) {
      appendFilterChip(
        chip.label,
        resourcesState.mallFilter === chip.id,
        () => {
          resourcesState.mallFilter = chip.id;
          renderToolsList();
        },
        { mallFilter: chip.id },
        toolFiltersEl,
      );
    }
    const items = filteredMallToolItems();
    toolsListEl.replaceChildren();
    toolsEmptyEl.hidden = items.length > 0;
    toolsEmptyEl.textContent = resourcesState.resourceQuery ? t("chat.resourcesNoMatch") : t("resources.emptyToolsStore");
    for (const item of items) {
      const entry = entryFromMallItem(item);
      if (entry) appendUnifiedSkillRow(toolsListEl, entry);
    }
    toolsFootnoteEl.textContent = resourcesState.skillsStatusMessage ?? defaultToolsFootnote();
    return;
  }

  toolFiltersEl.replaceChildren();
  appendFilterChip(
    t("resources.filterAll"),
    resourcesState.toolFilter === "all",
    () => {
      resourcesState.toolFilter = "all";
      renderToolsList();
    },
    { toolFilter: "all" },
    toolFiltersEl,
  );
  appendFilterChip(
    t("resources.filterIssue"),
    resourcesState.toolFilter === "issue",
    () => {
      resourcesState.toolFilter = "issue";
      renderToolsList();
    },
    { toolFilter: "issue" },
    toolFiltersEl,
  );

  const allRows = buildMcpRows();
  const rows = allRows.filter((row) => {
    if (resourcesState.toolFilter === "issue" && !row.issue) return false;
    return rowMatchesQuery(row);
  });

  toolsListEl.replaceChildren();
  toolsEmptyEl.hidden = rows.length > 0;
  toolsEmptyEl.textContent = resourcesState.resourceQuery ? t("chat.resourcesNoMatch") : t("resources.emptyTools");
  for (const row of rows) {
    appendResourceRow(toolsListEl, row, t("resources.toolBadge"));
  }
  toolsFootnoteEl.textContent = resourcesState.skillsStatusMessage ?? defaultToolsFootnote();
}

export function canInstallMallItem(item: TeamupsCatalogItem): boolean {
  return item.free || item.owned;
}

export function stopMallLoginPoll(): void {
  if (resourcesState.mallLoginTimer != null) {
    window.clearTimeout(resourcesState.mallLoginTimer);
    resourcesState.mallLoginTimer = null;
  }
}

export function renderMallAccount(): void {
  const signedIn = Boolean(resourcesState.lastTeamupsAccount?.signed_in);
  mallAccountEl.classList.toggle("is-signed-in", signedIn);
  mallLoginEl.hidden = signedIn;
  mallLogoutEl.hidden = !signedIn;
  mallLoginEl.disabled = resourcesState.mallLoginInFlight;
  mallLogoutEl.disabled = resourcesState.mallLoginInFlight;
  if (resourcesState.mallLoginInFlight) {
    mallAccountStatusEl.textContent = t("resources.mallLoggingIn");
  } else if (signedIn) {
    mallAccountStatusEl.textContent = t("resources.mallAccountSignedIn", {
      count: String(resourcesState.lastTeamupsAccount?.pack_count ?? 0),
    });
  } else {
    mallAccountStatusEl.textContent = t("resources.mallAccountSignedOut");
  }
}

export async function loadMall(): Promise<void> {
  try {
    const [catalog, account] = await Promise.all([
      listTeamupsMallCatalog(),
      teamupsAccountStatus().catch(() => null),
    ]);
    resourcesState.lastMallCatalog = catalog;
    resourcesState.lastTeamupsAccount = account;
    applyAccountOwnership();
    if (skillsPanelActive()) {
      renderResourcesList();
    }
  } catch (error) {
    resourcesState.lastMallCatalog = null;
    setSkillsFootnote(withErrorDetail(t("resources.mallLoadFailed"), error));
    renderMallAccount();
    renderSkillsList();
  }
}

export async function scheduleMallLoginPoll(
  deviceCode: string,
  intervalSec: number,
  expiresAt: number,
): Promise<void> {
  stopMallLoginPoll();
  const tick = async () => {
    if (Date.now() >= expiresAt) {
      resourcesState.mallLoginInFlight = false;
      renderMallAccount();
      setSkillsFootnote(t("resources.mallLoginExpired"));
      return;
    }
    try {
      const poll = await pollTeamupsLogin({
        deviceCode,
      });
      if (poll.status === "pending") {
        resourcesState.mallLoginTimer = window.setTimeout(() => {
          void tick();
        }, Math.max(1, intervalSec) * 1000);
        return;
      }
      resourcesState.mallLoginInFlight = false;
      if (poll.status === "approved") {
        setSkillsFootnote(t("resources.mallLoginOk"));
        await loadMall();
        return;
      }
      if (poll.status === "expired" || poll.status === "denied") {
        setSkillsFootnote(t("resources.mallLoginExpired"));
        renderMallAccount();
        return;
      }
    } catch (error) {
      resourcesState.mallLoginInFlight = false;
      setSkillsFootnote(teamupsLoginFailure(error));
      renderMallAccount();
    }
  };
  await tick();
}

export async function startMallLogin(): Promise<void> {
  if (resourcesState.mallLoginInFlight) return;
  resourcesState.mallLoginInFlight = true;
  renderMallAccount();
  setSkillsFootnote(t("resources.mallLoggingIn"));
  try {
    const started = await startTeamupsLogin();
    await openUrl(started.verification_url);
    const expiresAt = Date.now() + Math.max(30, started.expires_in_sec) * 1000;
    await scheduleMallLoginPoll(started.device_code, started.interval_sec, expiresAt);
  } catch (error) {
    resourcesState.mallLoginInFlight = false;
    setSkillsFootnote(teamupsLoginFailure(error));
    renderMallAccount();
  }
}

export async function signOutMallAccount(): Promise<void> {
  if (resourcesState.mallLoginInFlight) return;
  stopMallLoginPoll();
  try {
    resourcesState.lastTeamupsAccount = await signOutTeamups();
    setSkillsFootnote(t("resources.mallLogoutOk"));
    await loadMall();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("resources.mallLoginFailed"), error));
  }
}

export async function openMallPurchase(item: TeamupsCatalogItem): Promise<void> {
  const url = item.purchase_url?.trim();
  if (!url) {
    setSkillsFootnote(t("resources.mallInstallFailed"));
    return;
  }
  try {
    await openUrl(url);
    resourcesState.skillsStatusMessage = null;
    syncSkillsFootnote();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("resources.mallInstallFailed"), error));
  }
}

export async function installMallItem(
  item: TeamupsCatalogItem,
  button: HTMLButtonElement,
): Promise<void> {
  if (resourcesState.mallActionInFlight) return;
  resourcesState.mallActionInFlight = true;
  button.disabled = true;
  const previous = button.textContent;
  button.textContent = t("resources.mallInstalling");
  setSkillsFootnote(t("resources.mallInstalling"));
  try {
    const report = await installTeamupsMallItem({
      kind: item.kind,
      id: item.id,
      packSlug:
        item.kind === "skill"
          ? (item.pack_slug?.trim() || item.id)
          : item.pack_slug,
    });
    setSkillsFootnote(t("resources.mallInstallOk", {
      installed: String(report.installed),
    }));
    if (resourcesState.activeSection === "skills") {
      resourcesState.skillFilter = "issue";
      resourcesState.highlightSkillKey = item.kind === "skill" ? item.id : null;
    }
    await Promise.all([loadMall(), loadSkills(), loadMcpStatus()]);
  } catch (error) {
    setSkillsFootnote(teamupsMallInstallFailure(error));
    button.disabled = false;
    button.textContent = previous;
  } finally {
    resourcesState.mallActionInFlight = false;
  }
}
