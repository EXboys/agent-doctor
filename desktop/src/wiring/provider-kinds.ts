import { isTeamEdition } from "../edition";
import { t } from "../i18n";

const tablist = document.querySelector<HTMLElement>("#provider-kind-tabs");
const tabs = Array.from(
  document.querySelectorAll<HTMLButtonElement>("#provider-kind-tabs [data-provider-kind]"),
);

function paneFor(kind: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(`#provider-kind-${kind}`);
}

function visibleTabs(): HTMLButtonElement[] {
  return tabs.filter((tab) => !tab.hidden);
}

export function selectProviderKind(kind: string): void {
  const visible = visibleTabs();
  const next = visible.some((tab) => tab.dataset.providerKind === kind) ? kind : "models";
  for (const tab of tabs) {
    const on = tab.dataset.providerKind === next && !tab.hidden;
    tab.classList.toggle("is-active", on);
    tab.setAttribute("aria-selected", on ? "true" : "false");
    tab.tabIndex = on ? 0 : -1;
    const pane = tab.dataset.providerKind ? paneFor(tab.dataset.providerKind) : null;
    if (pane) pane.hidden = !on;
  }
}

/** Team builds have no local-compute card, so that tab stays out of the row. */
export function syncProviderKindTabs(): void {
  const localTab = tabs.find((tab) => tab.dataset.providerKind === "local");
  if (localTab) localTab.hidden = isTeamEdition();
  if (tablist) tablist.setAttribute("aria-label", t("providerKind.tabs"));
  const active = tabs.find((tab) => tab.classList.contains("is-active") && !tab.hidden);
  selectProviderKind(active?.dataset.providerKind || "models");
}

export function bindProviderKindTabs(): void {
  if (!tablist) return;
  for (const tab of tabs) {
    tab.addEventListener("click", () => {
      const kind = tab.dataset.providerKind;
      if (kind) selectProviderKind(kind);
    });
  }
  tablist.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
    const visible = visibleTabs();
    const index = visible.findIndex((tab) => tab.classList.contains("is-active"));
    if (index < 0 || visible.length === 0) return;
    event.preventDefault();
    const step = event.key === "ArrowRight" ? 1 : -1;
    const next = visible[(index + step + visible.length) % visible.length];
    const kind = next?.dataset.providerKind;
    if (!kind || !next) return;
    selectProviderKind(kind);
    next.focus();
  });
  syncProviderKindTabs();
}
