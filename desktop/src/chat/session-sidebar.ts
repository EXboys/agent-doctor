import type { ChatSession } from "./types";

/** Default visible chats per project before "More". */
export const SESSIONS_PREVIEW_COUNT = 3;

export function sidebarGroupKey(name: string | null): string {
  return name ?? "__none__";
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
