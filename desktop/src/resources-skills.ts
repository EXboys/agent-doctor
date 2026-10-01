
import { ask } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import type { MessageKey } from "./i18n";
import { withErrorDetail } from "./friendly-error";
import { classifySkillCategory, skillCategoryLabelKey, SKILL_CATEGORY_ORDER } from "./skill-categories";
import type { SkillCategoryId } from "./skill-categories";
import { agentFilterRuntimeIds, runtimeLabel, skillMountRuntimeIds } from "./runtime-catalog";
import type { ResourceRow, SkillAgentUsage, SkillInventoryItem, SkillsInventoryReport, TeamupsCatalogItem } from "./types";
import { resourcesState, skillsPanelEl, skillFiltersEl, agentBarEl, agentFiltersEl, agentHintEl, searchEl, listEl, emptyEl, personalEdition } from "./resources-state";
import type { SkillFilter, MallFilter, UnifiedSkillEntry } from "./resources-state";
import { renderResourcesList, setSection, setSkillsFootnote, skillsPanelActive, syncSkillsFootnote } from "./resources";
import { withCatalogInstallState } from "./resources-agents";
import { canInstallMallItem, filteredMallItems, installMallItem, isMallToolItem, openMallPurchase, renderMallAccount, startMallLogin } from "./resources-mall";
import { unmountSyncedSkills, mountSyncedSkills, listSkillsInventory, skillMountRuntimeIds as fetchSkillMountRuntimeIds } from "./ipc";

export function agentChipLabel(runtime: string): string {
  return runtimeLabel(runtime);
}

export function entryMountedOn(entry: UnifiedSkillEntry, runtime: string): boolean {
  return Boolean(entry.agents?.some((agent) => agent.runtime === runtime && agent.mounted));
}

/** Agents on this computer that can receive a skill. Inventory is install-filtered. */
export function mountTargetRuntimeIds(): string[] {
  const installed = resourcesState.lastSkillsInventory?.available_mount_runtimes;
  if (installed) return installed;
  return skillMountRuntimeIds();
}

export function mergeSkillAgents(skill: SkillInventoryItem): SkillAgentUsage[] {
  const fromApi = new Map(skill.agents.map((agent) => [agent.runtime, agent]));
  return mountTargetRuntimeIds().map(
    (runtime) =>
      fromApi.get(runtime) ?? {
        runtime,
        scope: "not mounted",
        path: "",
        mounted: false,
      },
  );
}

export function localSkillById(skillId: string) {
  return resourcesState.lastSkillsInventory?.skills.find((s) => s.skill_id === skillId);
}

export function joinedAgentNames(agents: SkillAgentUsage[]): string {
  return agents.map((agent) => runtimeLabel(agent.runtime)).join("、");
}

export function entryFromLocalSkill(skill: NonNullable<SkillsInventoryReport["skills"][number]>): UnifiedSkillEntry {
  const agents = mergeSkillAgents(skill);
  const mounted = agents.filter((a) => a.mounted).length;
  const totalAgents = agents.length;
  const needsMount = totalAgents > 0 && mounted === 0;
  const anyUnmounted = totalAgents > 0 && mounted < totalAgents;
  const category = classifySkillCategory({
    id: skill.skill_id,
    name: skill.name,
    description: skill.description,
  });
  const mallItem = resourcesState.lastMallCatalog?.items.find(
    (item) => item.kind === "skill" && item.id === skill.skill_id,
  );
  return {
    key: skill.skill_id,
    name: skill.name || skill.skill_id,
    description: skill.description?.trim() || "",
    category,
    badgeLabel: t(skillCategoryLabelKey(category) as MessageKey),
    sub: "",
    meta: "",
    tone: needsMount ? "warn" : anyUnmounted ? "warn" : "ok",
    issue: needsMount,
    skillId: skill.skill_id,
    needsMount: anyUnmounted,
    canUnmount: mounted > 0,
    mallItem,
    storeOnly: false,
    iconKind: "skill",
    agents,
  };
}

export function storeIssue(item: TeamupsCatalogItem): boolean {
  return !item.installed && item.owned;
}

export function isStoreScope(): boolean {
  return personalEdition && resourcesState.skillFilter === "store";
}

export function mallItemPriceMeta(item: TeamupsCatalogItem): string {
  const bits: string[] = [];
  if (item.installed) {
    bits.push(t("resources.mallInstalled"));
  } else if (item.free) {
    bits.push(t("resources.mallFree"));
  } else if (item.price_label) {
    bits.push(item.price_label);
  }
  if (item.skill_count != null) {
    bits.push(t("resources.mallSkillCount", { count: String(item.skill_count) }));
  }
  if (item.owned && !item.free && !item.installed) {
    bits.push(t("resources.mallOwned"));
  }
  return bits.join(" · ");
}

export function entryFromMallItem(item: TeamupsCatalogItem): UnifiedSkillEntry | null {
  if (item.kind === "skill") {
    const local = localSkillById(item.id);
    if (local) return entryFromLocalSkill(local);
  }
  const category: SkillCategoryId = "other";
  const badgeLabel =
    item.kind === "pack"
      ? t("resources.mallPackBadge")
      : isMallToolItem(item)
        ? t("resources.toolBadge")
        : t("resources.mallSkillBadge");
  const issue = storeIssue(item);
  return {
    key: `store:${item.kind}:${item.id}`,
    name: item.name || item.id,
    description: item.description.trim() || item.id,
    category,
    badgeLabel,
    sub: badgeLabel,
    meta: mallItemPriceMeta(item),
    tone: item.installed ? "ok" : issue ? "warn" : "muted",
    issue,
    mallItem: item,
    storeOnly: true,
    iconKind: item.kind === "pack" ? "pack" : isMallToolItem(item) ? "mcp" : "skill",
  };
}

export function buildLocalSkillEntries(): UnifiedSkillEntry[] {
  const rows: UnifiedSkillEntry[] = [];
  for (const skill of resourcesState.lastSkillsInventory?.skills ?? []) {
    rows.push(entryFromLocalSkill(skill));
  }
  return rows.sort((a, b) => a.name.localeCompare(b.name));
}

export function buildStoreSkillEntries(): UnifiedSkillEntry[] {
  const rows: UnifiedSkillEntry[] = [];
  for (const item of filteredMallItems()) {
    const entry = entryFromMallItem(item);
    if (entry) rows.push(entry);
  }
  return rows;
}

export function buildUnifiedSkillEntries(): UnifiedSkillEntry[] {
  if (resourcesState.skillFilter === "store" && personalEdition) {
    return buildStoreSkillEntries();
  }
  if (resourcesState.skillFilter === "issue" && personalEdition) {
    const localIssues = buildLocalSkillEntries().filter((entry) => entry.issue);
    const storeIssues: UnifiedSkillEntry[] = [];
    for (const item of resourcesState.lastMallCatalog?.items ?? []) {
      if (!storeIssue(item)) continue;
      const entry = entryFromMallItem(item);
      if (entry?.storeOnly) storeIssues.push(entry);
    }
    return [...localIssues, ...storeIssues];
  }
  const local = buildLocalSkillEntries();
  if (!personalEdition || resourcesState.skillFilter !== "all" || resourcesState.resourceQuery) {
    return local;
  }
  const localIds = new Set(local.map((row) => row.skillId).filter(Boolean));
  const storeOnly: UnifiedSkillEntry[] = [];
  for (const item of resourcesState.lastMallCatalog?.items ?? []) {
    if (item.kind === "skill" && (item.installed || localIds.has(item.id))) continue;
    if (item.kind === "pack" && item.installed) continue;
    const entry = entryFromMallItem(item);
    if (entry?.storeOnly) storeOnly.push(entry);
  }
  return [...local, ...storeOnly];
}

export function entryMatchesQuery(entry: UnifiedSkillEntry): boolean {
  if (!resourcesState.resourceQuery) return true;
  const blob = `${entry.name} ${entry.description} ${entry.sub} ${entry.meta} ${entry.skillId ?? ""} ${
    entry.mallItem?.id ?? ""
  }`.toLowerCase();
  return blob.includes(resourcesState.resourceQuery);
}

export function skillDescription(skillId: string | undefined): string {
  if (!skillId) return "";
  return resourcesState.lastSkillsInventory?.skills.find((s) => s.skill_id === skillId)?.description?.trim() || "";
}

export function appendResourceRow(parent: HTMLElement, row: ResourceRow, tagLabel: string): void {
  const li = document.createElement("li");
  li.className = "res-catalog-item";

  const icon = document.createElement("span");
  icon.className = "res-catalog-icon";
  icon.classList.add(
    row.action === "open-browser" ? "is-browser" : row.kind === "skill" ? "is-skill" : "is-mcp",
  );
  icon.textContent = (row.name.trim().charAt(0) || "?").toUpperCase();

  const body = document.createElement("div");
  body.className = "res-catalog-body";
  const titleRow = document.createElement("div");
  titleRow.className = "res-catalog-title-row";
  const strong = document.createElement("strong");
  strong.textContent = row.name;
  const badge = document.createElement("span");
  badge.className = "res-catalog-badge";
  badge.textContent = tagLabel;
  titleRow.append(strong, badge);
  const desc = document.createElement("div");
  desc.className = "res-catalog-desc";
  desc.textContent = row.kind === "skill" ? skillDescription(row.skillId) || row.sub : row.sub;
  body.append(titleRow, desc);

  const metaWrap = document.createElement("div");
  metaWrap.className = "res-catalog-meta";
  const meta = document.createElement("span");
  meta.className = `tone-${row.tone}`;
  meta.textContent = row.meta;
  metaWrap.appendChild(meta);

  if (row.kind === "skill" && row.needsMount && row.skillId) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "btn-secondary btn-compact";
    btn.textContent = t("resources.mount");
    const skillId = row.skillId;
    btn.addEventListener("click", () => {
      void mountSkill(skillId);
    });
    metaWrap.appendChild(btn);
  }

  if (row.action === "open-browser") {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "res-catalog-link";
    btn.textContent = row.tone === "ok" ? t("resources.toolBrowserView") : t("resources.toolBrowserOpen");
    const openBrowser = () => setSection("browser");
    btn.addEventListener("click", (event) => {
      event.stopPropagation();
      openBrowser();
    });
    metaWrap.appendChild(btn);
    li.classList.add("is-link");
    li.tabIndex = 0;
    li.addEventListener("click", openBrowser);
    li.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        openBrowser();
      }
    });
  }

  li.append(icon, body, metaWrap);
  parent.appendChild(li);
}

export function appendFilterChip(
  label: string,
  active: boolean,
  onClick: () => void,
  dataset: Record<string, string>,
  parent: HTMLElement = skillFiltersEl,
  count?: number,
): void {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = `filter-chip${active ? " is-active" : ""}`;
  for (const [key, value] of Object.entries(dataset)) {
    btn.dataset[key] = value;
  }
  if (count == null) {
    btn.textContent = label;
  } else {
    const name = document.createElement("span");
    name.textContent = label;
    const num = document.createElement("span");
    num.className = "chip-count";
    num.textContent = String(count);
    btn.append(name, num);
  }
  btn.addEventListener("click", onClick);
  parent.appendChild(btn);
}

export function installedSkillAgents(): string[] {
  const installed = new Set<string>();
  for (const runtime of resourcesState.lastDoctorReport?.runtimes ?? []) {
    if (withCatalogInstallState(runtime).installed) installed.add(runtime.id);
  }
  if (resourcesState.cursorOnThisComputer) installed.add("cursor");
  return agentFilterRuntimeIds().filter((id) => installed.has(id));
}

export function renderSkillFilters(): void {
  const local = buildLocalSkillEntries();
  const storeItems = resourcesState.lastMallCatalog?.items ?? [];
  const issueCount =
    local.filter((row) => row.issue).length +
    (personalEdition ? storeItems.filter((item) => storeIssue(item)).length : 0);

  if (
    !isStoreScope() &&
    resourcesState.skillFilter !== "all" &&
    resourcesState.skillFilter !== "issue" &&
    !SKILL_CATEGORY_ORDER.includes(resourcesState.skillFilter as SkillCategoryId)
  ) {
    resourcesState.skillFilter = "all";
  }

  const showStoreChrome = isStoreScope();
  skillFiltersEl.replaceChildren();

  if (showStoreChrome) {
    agentBarEl.hidden = true;
    renderMallAccount();
    const mallChips: Array<{ id: MallFilter; label: string }> = [
      { id: "all", label: t("resources.filterAll") },
      { id: "free", label: t("resources.mallFilterFree") },
      { id: "paid", label: t("resources.mallFilterPaid") },
      { id: "pack", label: t("resources.mallFilterPack") },
      { id: "skill", label: t("resources.mallFilterSkill") },
    ];
    for (const chip of mallChips) {
      appendFilterChip(chip.label, resourcesState.mallFilter === chip.id, () => {
        resourcesState.mallFilter = chip.id;
        renderSkillsList();
      }, { mallFilter: chip.id });
    }
    return;
  }

  renderAgentBar(local);
  const scoped =
    resourcesState.agentFilter === "all" ? local : local.filter((row) => entryMountedOn(row, resourcesState.agentFilter));
  const scopedIssue =
    resourcesState.agentFilter === "all" ? issueCount : scoped.filter((row) => row.issue).length;

  const counts = new Map<SkillFilter, number>();
  counts.set("all", scoped.length);
  counts.set("issue", scopedIssue);
  for (const id of SKILL_CATEGORY_ORDER) {
    counts.set(id, scoped.filter((row) => row.category === id).length);
  }

  const chips: Array<{ id: SkillFilter; label: string; count: number }> = [
    { id: "all", label: t("resources.filterAll"), count: counts.get("all") ?? 0 },
    { id: "issue", label: t("resources.filterIssue"), count: counts.get("issue") ?? 0 },
  ];
  for (const id of SKILL_CATEGORY_ORDER) {
    const count = counts.get(id) ?? 0;
    if (id !== "other" && count === 0) continue;
    chips.push({
      id,
      label: t(skillCategoryLabelKey(id) as MessageKey),
      count,
    });
  }

  if (!chips.some((chip) => chip.id === resourcesState.skillFilter)) {
    resourcesState.skillFilter = "all";
  }

  const navActive = resourcesState.skillFilter === "issue" ? "issue" : resourcesState.skillNavActive;
  for (const chip of chips) {
    if (chip.id === "issue" && chip.count === 0) continue;
    const active =
      chip.id === "issue" ? navActive === "issue" : navActive === chip.id && resourcesState.skillFilter !== "issue";
    appendFilterChip(`${chip.label} ${chip.count}`, active, () => {
      void onSkillNavClick(chip.id);
    }, { skillFilter: chip.id });
  }
}

export function renderAgentBar(local: UnifiedSkillEntry[]): void {
  const agents = installedSkillAgents();
  if (resourcesState.agentFilter !== "all" && !agents.includes(resourcesState.agentFilter)) {
    resourcesState.agentFilter = "all";
  }
  if (agents.length < 2) {
    agentBarEl.hidden = true;
    agentHintEl.hidden = true;
    agentHintEl.replaceChildren();
    return;
  }
  agentBarEl.hidden = false;
  agentFiltersEl.replaceChildren();
  appendFilterChip(
    t("resources.agentFilterAll"),
    resourcesState.agentFilter === "all",
    () => {
      if (resourcesState.agentFilter === "all") return;
      setAgentFilter("all");
    },
    { agentFilter: "all" },
    agentFiltersEl,
  );
  for (const runtime of agents) {
    const count = local.filter((entry) => entryMountedOn(entry, runtime)).length;
    appendFilterChip(
      agentChipLabel(runtime),
      resourcesState.agentFilter === runtime,
      () => {
        setAgentFilter(resourcesState.agentFilter === runtime ? "all" : runtime);
      },
      { agentFilter: runtime },
      agentFiltersEl,
      count,
    );
  }

  if (resourcesState.agentFilter === "all") {
    agentHintEl.hidden = true;
    agentHintEl.replaceChildren();
    return;
  }
  const count = local.filter((entry) => entryMountedOn(entry, resourcesState.agentFilter)).length;
  agentHintEl.hidden = false;
  agentHintEl.replaceChildren();
  const text = document.createElement("span");
  text.textContent = t("resources.agentFilterHint", {
    agent: agentChipLabel(resourcesState.agentFilter),
    count: String(count),
  });
  const clear = document.createElement("button");
  clear.type = "button";
  clear.className = "resources-agent-clear";
  clear.textContent = t("resources.agentFilterClear");
  clear.addEventListener("click", () => setAgentFilter("all"));
  agentHintEl.append(text, clear);
}

export function paintSkillNav(active: SkillFilter): void {
  resourcesState.skillNavActive = active;
  skillFiltersEl.querySelectorAll<HTMLButtonElement>("[data-skill-filter]").forEach((chip) => {
    chip.classList.toggle("is-active", chip.dataset.skillFilter === active);
  });
  const current = skillFiltersEl.querySelector<HTMLButtonElement>(`[data-skill-filter="${active}"]`);
  current?.scrollIntoView({ block: "nearest", inline: "center", behavior: "smooth" });
}

export function setAgentFilter(runtime: string): void {
  if (resourcesState.agentFilter === runtime) return;
  resourcesState.agentFilter = runtime;
  resourcesState.skillsStatusMessage = null;
  renderResourcesList();
  scrollSkillsPanelTo(0);
}

export function lockSkillScrollSpy(): void {
  resourcesState.skillScrollLock = true;
  if (resourcesState.skillScrollUnlockTimer != null) window.clearTimeout(resourcesState.skillScrollUnlockTimer);
  resourcesState.skillScrollUnlockTimer = window.setTimeout(() => {
    resourcesState.skillScrollLock = false;
    resourcesState.skillScrollUnlockTimer = null;
  }, 500);
}

export function scrollSkillsPanelTo(top: number): void {
  lockSkillScrollSpy();
  skillsPanelEl.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
}

export function scrollToSkillGroup(id: SkillFilter): void {
  if (id === "all") {
    scrollSkillsPanelTo(0);
    paintSkillNav("all");
    return;
  }
  const heading = listEl.querySelector<HTMLElement>(`[data-skill-group="${id}"]`);
  if (!heading) return;
  const offset = 8;
  const top = heading.getBoundingClientRect().top - skillsPanelEl.getBoundingClientRect().top
    + skillsPanelEl.scrollTop
    - offset;
  paintSkillNav(id);
  scrollSkillsPanelTo(top);
}

export async function onSkillNavClick(id: SkillFilter): Promise<void> {
  resourcesState.skillsStatusMessage = null;
  if (id === "issue") {
    resourcesState.skillFilter = "issue";
    renderResourcesList();
    scrollSkillsPanelTo(0);
    return;
  }
  const needRebuild = resourcesState.skillFilter !== "all" || Boolean(resourcesState.resourceQuery);
  if (resourcesState.resourceQuery) {
    resourcesState.resourceQuery = "";
    searchEl.value = "";
  }
  resourcesState.skillFilter = "all";
  if (needRebuild) renderResourcesList();
  requestAnimationFrame(() => {
    scrollToSkillGroup(id);
  });
}

export function bindSkillGroupSpy(): void {
  resourcesState.skillGroupObserver?.disconnect();
  resourcesState.skillGroupObserver = null;
  if (resourcesState.skillFilter !== "all" || resourcesState.resourceQuery || isStoreScope()) return;
  const headings = [...listEl.querySelectorAll<HTMLElement>("[data-skill-group]")];
  if (headings.length === 0) return;

  resourcesState.skillGroupObserver = new IntersectionObserver(
    (entries) => {
      if (resourcesState.skillScrollLock) return;
      const visible = entries
        .filter((entry) => entry.isIntersecting)
        .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      const topMost = visible[0]?.target;
      const group = topMost instanceof HTMLElement ? topMost.dataset.skillGroup : undefined;
      if (!group) {
        if (skillsPanelEl.scrollTop < 24) paintSkillNav("all");
        return;
      }
      paintSkillNav(group as SkillFilter);
    },
    {
      root: skillsPanelEl,
      rootMargin: "-20% 0px -65% 0px",
      threshold: 0,
    },
  );
  for (const heading of headings) resourcesState.skillGroupObserver.observe(heading);
}

export function onSkillsPanelScroll(): void {
  if (resourcesState.skillScrollLock) return;
  if (skillsPanelEl.scrollTop < 20) paintSkillNav("all");
}

export function appendSkillAgentChips(body: HTMLElement, entry: UnifiedSkillEntry): void {
  if (entry.storeOnly || !entry.skillId) return;
  const agents = entry.agents ?? [];
  if (agents.length === 0) return;
  const row = document.createElement("div");
  row.className = "res-skill-agents";
  const label = document.createElement("span");
  label.className = "res-skill-agents-label";
  label.textContent = t("resources.skillAgentsLabel");
  row.appendChild(label);

  const chips = document.createElement("div");
  chips.className = "res-skill-agents-chips";
  for (const agent of agents) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = agent.mounted ? "skills-runtime is-on" : "skills-runtime";
      const runtimeLabelText = runtimeLabel(agent.runtime);
      chip.title = agent.mounted
        ? t("skills.unmountRuntime", { runtime: runtimeLabelText })
        : t("skills.mountRuntime", { runtime: runtimeLabelText });
      chip.setAttribute("aria-pressed", agent.mounted ? "true" : "false");
      const dot = document.createElement("i");
      dot.className = "skills-runtime-dot";
      dot.setAttribute("aria-hidden", "true");
      const name = document.createElement("span");
      name.textContent = runtimeLabelText;
      chip.append(dot, name);
      const skillId = entry.skillId;
      const runtime = agent.runtime;
      chip.addEventListener("click", () => {
        void toggleSkillRuntimeMount(chip, skillId, runtime, chip.classList.contains("is-on"));
      });
      chips.appendChild(chip);
  }
  row.append(chips);
  body.appendChild(row);
}

export async function toggleSkillRuntimeMount(
  chip: HTMLButtonElement,
  skillId: string,
  runtime: string,
  wasMounted: boolean,
): Promise<void> {
  if (chip.classList.contains("is-busy")) return;
  chip.classList.add("is-busy");
  setSkillsFootnote(wasMounted ? t("skills.unmounting") : t("skills.mounting"));
  try {
    const report = wasMounted
      ? await unmountSyncedSkills({
          skillIds: [skillId],
          runtimes: [runtime],
        })
      : await mountSyncedSkills({
          skillIds: [skillId],
          runtimes: [runtime],
        });
    setSkillsFootnote(
      wasMounted
        ? t("skills.unmountOk", {
            unmounted: String(report.unmounted),
            skipped: String(report.skipped),
            failed: String(report.failed),
          })
        : t("skills.mountOk", {
            mounted: String(report.mounted),
            skipped: String(report.skipped),
            failed: String(report.failed),
          }),
    );
    await loadSkills();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("skills.mountFailed"), error));
  } finally {
    chip.classList.remove("is-busy");
  }
}

export function appendUnifiedSkillActions(metaWrap: HTMLElement, entry: UnifiedSkillEntry): void {
  if (entry.skillId && entry.agents && entry.agents.length > 0) {
    const skillId = entry.skillId;
    if (entry.needsMount) {
      const missing = entry.agents.filter((agent) => !agent.mounted);
      const list = joinedAgentNames(missing);
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn-primary btn-compact";
      btn.textContent = t("resources.mountAllAgents");
      btn.title = t("resources.mountAllAgentsHint", { list });
      btn.addEventListener("click", () => {
        void mountSkill(
          skillId,
          missing.map((agent) => agent.runtime),
        );
      });
      metaWrap.appendChild(btn);
    }
    if (entry.canUnmount) {
      const removeBtn = document.createElement("button");
      removeBtn.type = "button";
      removeBtn.className = "btn-ghost btn-compact";
      removeBtn.textContent = t("resources.removeFromAgents");
      removeBtn.title = t("resources.removeFromAgentsHint");
      removeBtn.addEventListener("click", () => {
        void unmountSkill(skillId, entry.name);
      });
      metaWrap.appendChild(removeBtn);
    }
    return;
  }

  if (entry.skillId && !entry.storeOnly) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "btn-secondary btn-compact";
    btn.textContent = t("resources.goInstallAgent");
    btn.addEventListener("click", () => {
      setSection("agents");
    });
    metaWrap.appendChild(btn);
    return;
  }

  const item = entry.mallItem;
  if (!item || !entry.storeOnly) return;

  if (item.installed) {
    const done = document.createElement("span");
    done.className = "tone-ok";
    done.textContent = t("resources.mallInstalled");
    metaWrap.appendChild(done);
    return;
  }
  if (canInstallMallItem(item)) {
    const installBtn = document.createElement("button");
    installBtn.type = "button";
    installBtn.className = "btn-primary btn-compact";
    installBtn.textContent = t("resources.mallInstall");
    installBtn.addEventListener("click", () => {
      void installMallItem(item, installBtn);
    });
    metaWrap.appendChild(installBtn);
    return;
  }
  const buyBtn = document.createElement("button");
  buyBtn.type = "button";
  buyBtn.className = "btn-secondary btn-compact";
  buyBtn.textContent = resourcesState.lastTeamupsAccount?.signed_in
    ? t("resources.mallBuy")
    : t("resources.mallLogin");
  buyBtn.addEventListener("click", () => {
    if (resourcesState.lastTeamupsAccount?.signed_in) {
      void openMallPurchase(item);
    } else {
      void startMallLogin();
    }
  });
  metaWrap.appendChild(buyBtn);
}

export function appendUnifiedSkillRow(parent: HTMLElement, entry: UnifiedSkillEntry): void {
  const li = document.createElement("li");
  li.className = "res-catalog-item";
  if (entry.storeOnly) li.classList.add("is-store-item");
  li.dataset.skillKey = entry.key;
  if (resourcesState.highlightSkillKey && resourcesState.highlightSkillKey === entry.key) {
    li.classList.add("is-highlight");
  }

  const icon = document.createElement("span");
  icon.className = "res-catalog-icon";
  icon.classList.add(
    entry.iconKind === "pack" ? "is-pack" : entry.iconKind === "mcp" ? "is-mcp" : "is-skill",
  );
  icon.textContent = (entry.name.trim().charAt(0) || "?").toUpperCase();

  const body = document.createElement("div");
  body.className = "res-catalog-body";
  const titleRow = document.createElement("div");
  titleRow.className = "res-catalog-title-row";
  const strong = document.createElement("strong");
  strong.textContent = entry.name;
  const badge = document.createElement("span");
  badge.className = "res-catalog-badge";
  badge.textContent = entry.badgeLabel;
  titleRow.append(strong, badge);
  const desc = document.createElement("div");
  desc.className = "res-catalog-desc";
  desc.textContent = entry.description;
  if (!entry.description) desc.hidden = true;
  body.append(titleRow, desc);
  appendSkillAgentChips(body, entry);

  const metaWrap = document.createElement("div");
  metaWrap.className = "res-catalog-meta mall-actions";
  if (entry.meta) {
    const meta = document.createElement("span");
    meta.className = entry.storeOnly ? "res-catalog-store-price" : `tone-${entry.tone}`;
    meta.textContent = entry.meta;
    meta.title = entry.meta;
    metaWrap.appendChild(meta);
  }
  appendUnifiedSkillActions(metaWrap, entry);

  li.append(icon, body, metaWrap);
  bindCollapsibleRow(li, desc);
  parent.appendChild(li);
}

export function bindCollapsibleRow(li: HTMLElement, desc: HTMLElement): void {
  li.classList.add("is-collapsible");
  const more = document.createElement("button");
  more.type = "button";
  more.className = "res-catalog-more";
  more.hidden = true;
  desc.insertAdjacentElement("afterend", more);

  const syncMore = () => {
    const expanded = li.classList.contains("is-expanded");
    more.textContent = expanded ? t("resources.rowCollapse") : t("resources.rowExpand");
    if (expanded) {
      more.hidden = false;
      return;
    }
    const hasAgents = Boolean(li.querySelector(".res-skill-agents"));
    more.hidden = !hasAgents && desc.scrollHeight <= desc.clientHeight + 2;
  };

  const toggle = () => {
    li.classList.toggle("is-expanded");
    requestAnimationFrame(syncMore);
  };

  more.addEventListener("click", (event) => {
    event.stopPropagation();
    toggle();
  });
  li.addEventListener("click", (event) => {
    if ((event.target as HTMLElement).closest("button")) return;
    if (more.hidden && !li.classList.contains("is-expanded")) return;
    toggle();
  });
  requestAnimationFrame(syncMore);
}

export function renderSkillsList(): void {
  renderSkillFilters();

  let entries = buildUnifiedSkillEntries().filter((entry) => entryMatchesQuery(entry));

  if (resourcesState.skillFilter === "issue") {
    entries = entries.filter((entry) => entry.issue);
  } else if (resourcesState.skillFilter !== "all" && resourcesState.skillFilter !== "store") {
    entries = entries.filter((entry) => !entry.storeOnly && entry.category === resourcesState.skillFilter);
  }
  if (resourcesState.agentFilter !== "all") {
    entries = entries.filter((entry) => entryMountedOn(entry, resourcesState.agentFilter));
  }

  listEl.replaceChildren();
  const emptyCopy =
    resourcesState.skillFilter === "store"
      ? resourcesState.resourceQuery
        ? t("chat.resourcesNoMatch")
        : t("resources.emptyMall")
      : resourcesState.resourceQuery
        ? t("chat.resourcesNoMatch")
        : resourcesState.agentFilter !== "all"
          ? t("resources.emptyAgentSkills", { agent: agentChipLabel(resourcesState.agentFilter) })
          : t("resources.emptySkills");
  emptyEl.hidden = entries.length > 0;
  emptyEl.textContent = emptyCopy;

  if (resourcesState.skillFilter === "all" && !resourcesState.resourceQuery) {
    const localEntries = entries.filter((entry) => !entry.storeOnly);
    const storeEntries = entries.filter((entry) => entry.storeOnly);
    for (const category of SKILL_CATEGORY_ORDER) {
      const group = localEntries.filter((entry) => entry.category === category);
      if (group.length === 0) continue;
      const heading = document.createElement("h3");
      heading.className = "res-catalog-group";
      heading.dataset.skillGroup = category;
      heading.id = `skill-group-${category}`;
      heading.textContent = `${t(skillCategoryLabelKey(category) as MessageKey)} · ${group.length}`;
      listEl.appendChild(heading);
      const ul = document.createElement("ul");
      ul.className = "res-catalog-list";
      for (const entry of group) {
        appendUnifiedSkillRow(ul, entry);
      }
      listEl.appendChild(ul);
    }
    if (storeEntries.length > 0 && personalEdition) {
      const heading = document.createElement("h3");
      heading.className = "res-catalog-group";
      heading.dataset.skillGroup = "store";
      heading.id = "skill-group-store";
      heading.textContent = `${t("resources.groupStore")} · ${storeEntries.length}`;
      listEl.appendChild(heading);
      const ul = document.createElement("ul");
      ul.className = "res-catalog-list";
      for (const entry of storeEntries) {
        appendUnifiedSkillRow(ul, entry);
      }
      listEl.appendChild(ul);
    }
  } else {
    const ul = document.createElement("ul");
    ul.className = "res-catalog-list";
    for (const entry of entries) {
      appendUnifiedSkillRow(ul, entry);
    }
    listEl.appendChild(ul);
  }

  if (resourcesState.highlightSkillKey) {
    const target = listEl.querySelector<HTMLElement>(`[data-skill-key="${resourcesState.highlightSkillKey}"]`);
    target?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    resourcesState.highlightSkillKey = null;
  }

  bindSkillGroupSpy();
  syncSkillsFootnote();
}

export async function mountSkill(skillId: string, runtimes?: string[]): Promise<void> {
  const names = (runtimes ?? []).map((runtime) => runtimeLabel(runtime)).join("、");
  setSkillsFootnote(names ? t("resources.mountingNamed", { list: names }) : t("resources.installingToAgents"));
  try {
    const report = await mountSyncedSkills({
      skillIds: [skillId],
      runtimes: runtimes && runtimes.length > 0 ? runtimes : null,
    });
    setSkillsFootnote(
      report.failed === 0 && names
        ? t("resources.skillOnAgents", { list: names })
        : t("resources.installToAgentsOk", {
            mounted: String(report.mounted),
            skipped: String(report.skipped),
            failed: String(report.failed),
          }),
    );
    await loadSkills();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("resources.installToAgentsFailed"), error));
  }
}

export async function unmountSkill(skillId: string, name: string): Promise<void> {
  let ok = false;
  try {
    ok = await ask(t("resources.removeFromAgentsConfirm", { name }), {
      title: t("resources.removeFromAgents"),
      kind: "warning",
      okLabel: t("resources.removeFromAgents"),
      cancelLabel: t("resources.cancel"),
    });
  } catch {
    ok = window.confirm(t("resources.removeFromAgentsConfirm", { name }));
  }
  if (!ok) return;
  setSkillsFootnote(t("resources.removingFromAgents"));
  try {
    const report = await unmountSyncedSkills({
      skillIds: [skillId],
      runtimes: null,
    });
    setSkillsFootnote(t("resources.removeFromAgentsOk", {
      unmounted: String(report.unmounted),
      skipped: String(report.skipped),
      failed: String(report.failed),
    }));
    await loadSkills();
  } catch (error) {
    setSkillsFootnote(withErrorDetail(t("resources.removeFromAgentsFailed"), error));
  }
}

export async function loadSkills(): Promise<void> {
  try {
    resourcesState.lastSkillsInventory = await listSkillsInventory({
      remoteStats: false,
    });
    if (!resourcesState.lastSkillsInventory.available_mount_runtimes?.length) {
      try {
        resourcesState.lastSkillsInventory.available_mount_runtimes = await fetchSkillMountRuntimeIds();
      } catch {
        // Older desktop build — fall back to per-skill agents from the API.
      }
    }
  } catch {
    resourcesState.lastSkillsInventory = null;
  }
  if (skillsPanelActive()) {
    renderResourcesList();
  }
}
