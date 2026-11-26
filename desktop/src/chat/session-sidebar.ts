import { t } from "../i18n";
import type { ChatSession } from "./types";

/** Default visible chats per project before "More". */
export const SESSIONS_PREVIEW_COUNT = 3;

/** How many more chats one「更多」click reveals. */
export const SESSIONS_MORE_STEP = 5;

const SIDE_KEY = "agent-doctor-ask-side-collapsed";

function applySidebarCollapsed(collapsed: boolean): void {
  document.querySelector("#chat-shell")?.classList.toggle("is-side-collapsed", collapsed);
  try {
    localStorage.setItem(SIDE_KEY, collapsed ? "1" : "0");
  } catch {
    // The list still hides for this visit when storage is blocked.
  }
  syncSidebarCollapseLabels();
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
    // The button lives in the chat topbar and in each page header
    // (knowledge, schedule, skills), so match by class. On Windows and macOS
    // it stays visible and toggles; elsewhere it only appears once the list is hidden.
    if (target.closest(".chat-side-expand")) {
      event.preventDefault();
      const collapsed = document.querySelector("#chat-shell")?.classList.contains("is-side-collapsed") ?? false;
      const os = document.documentElement.dataset.os;
      const toggle = os === "windows" || os === "macos";
      applySidebarCollapsed(toggle ? !collapsed : false);
    }
  });
}

export function syncSidebarCollapseLabels(): void {
  const collapse = document.querySelector<HTMLButtonElement>("#chat-side-collapse");
  const expands = document.querySelectorAll<HTMLButtonElement>(".chat-side-expand");
  const hide = t("chat.sideCollapse");
  const show = t("chat.sideExpand");
  const listHidden = document.querySelector("#chat-shell")?.classList.contains("is-side-collapsed") ?? false;
  const os = document.documentElement.dataset.os;
  const pinned = os === "windows" || os === "macos";
  if (collapse) {
    collapse.title = hide;
    collapse.setAttribute("aria-label", hide);
  }
  const expandLabel = pinned && !listHidden ? hide : show;
  expands.forEach((expand) => {
    expand.title = expandLabel;
    expand.setAttribute("aria-label", expandLabel);
    expand.setAttribute("aria-pressed", pinned && !listHidden ? "true" : "false");
  });
}

export function sidebarGroupKey(name: string | null): string {
  return name ?? "__none__";
}

/**
 * Order key for one chat in a project.
 * Opening a chat from the island or an agent card surfaces it above older rows until a later update.
 */
export function sidebarSessionRank(updatedAt: number, surfacedAt = 0): number {
  return Math.max(updatedAt, surfacedAt);
}

export function visibleSessionsForGroup(
  sessions: ChatSession[],
  shown: number,
): { visible: ChatSession[]; hiddenCount: number } {
  const limit = Math.max(SESSIONS_PREVIEW_COUNT, shown);
  if (sessions.length <= limit) {
    return { visible: sessions, hiddenCount: 0 };
  }
  return {
    visible: sessions.slice(0, limit),
    hiddenCount: sessions.length - limit,
  };
}

/** Folder groups once the app has registered projects (incl. default agent-doctor). */
export function shouldUseProjectSidebar(workspaceCount: number): boolean {
  return workspaceCount > 0;
}
