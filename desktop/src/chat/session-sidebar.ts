import { t } from "../i18n";
import type { ChatSession } from "./types";

/** Default visible chats per project before "More". */
export const SESSIONS_PREVIEW_COUNT = 3;

const SIDE_KEY = "agent-doctor-ask-side-collapsed";

function applySidebarCollapsed(collapsed: boolean): void {
  document.querySelector("#chat-shell")?.classList.toggle("is-side-collapsed", collapsed);
  try {
    localStorage.setItem(SIDE_KEY, collapsed ? "1" : "0");
  } catch {
    // The list still hides for this visit when storage is blocked.
  }
}

export function bindSidebarCollapse(): void {
  applySidebarCollapsed(localStorage.getItem(SIDE_KEY) === "1");
  document.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    if (target.closest("#chat-side-collapse")) {
      event.preventDefault();
      applySidebarCollapsed(true);
      return;
    }
    if (target.closest("#chat-side-expand")) {
      event.preventDefault();
      applySidebarCollapsed(false);
    }
  });
}

export function syncSidebarCollapseLabels(): void {
  const collapse = document.querySelector<HTMLButtonElement>("#chat-side-collapse");
  const expand = document.querySelector<HTMLButtonElement>("#chat-side-expand");
  const hide = t("chat.sideCollapse");
  const show = t("chat.sideExpand");
  if (collapse) {
    collapse.title = hide;
    collapse.setAttribute("aria-label", hide);
  }
  if (expand) {
    expand.title = show;
    expand.setAttribute("aria-label", show);
  }
}

export function sidebarGroupKey(name: string | null): string {
  return name ?? "__none__";
}

/**
 * Order key for one chat in a project.
 * Opening a chat from the island surfaces it above older rows until a later update.
 */
export function sidebarSessionRank(updatedAt: number, surfacedAt = 0): number {
  return Math.max(updatedAt, surfacedAt);
}

export function visibleSessionsForGroup(
  sessions: ChatSession[],
  groupKey: string,
  expandedLists: Set<string>,
): { visible: ChatSession[]; hiddenCount: number } {
  if (sessions.length <= SESSIONS_PREVIEW_COUNT || expandedLists.has(groupKey)) {
    return { visible: sessions, hiddenCount: 0 };
  }
  return {
    visible: sessions.slice(0, SESSIONS_PREVIEW_COUNT),
    hiddenCount: sessions.length - SESSIONS_PREVIEW_COUNT,
  };
}

/** Folder groups once the app has registered projects (incl. default agent-doctor). */
export function shouldUseProjectSidebar(workspaceCount: number): boolean {
  return workspaceCount > 0;
}
