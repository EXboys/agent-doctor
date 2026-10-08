import type { WorkspaceDoc } from "../ask-resources";
import type { ChatSession } from "./types";
import { orderedProjectNames } from "./project-order";

/** Registered workspace name for this chat (falls back to global default). */
export function sessionWorkspaceName(
  session: ChatSession,
  doc: WorkspaceDoc | null,
): string | null {
  const pinned = session.workspaceName?.trim();
  if (pinned) return pinned;
  const active = doc?.active?.trim();
  if (active && doc?.workspaces[active]) return active;
  if (!doc) return null;
  const names = Object.keys(doc.workspaces).sort();
  return names[0] ?? null;
}

export function sessionWorkspacePath(
  session: ChatSession,
  doc: WorkspaceDoc | null,
): string | null {
  const name = sessionWorkspaceName(session, doc);
  if (!name) return null;
  const path = doc?.workspaces[name]?.path?.trim();
  return path || null;
}

export function defaultWorkspaceName(doc: WorkspaceDoc | null): string | null {
  if (!doc) return null;
  const active = doc.active?.trim();
  if (active && doc.workspaces[active]) return active;
  const names = Object.keys(doc.workspaces).sort();
  return names[0] ?? null;
}

export type SessionWorkspaceGroup = {
  name: string | null;
  label: string;
  sessions: ChatSession[];
};

/** Sidebar order: named workspaces (sorted), then chats without a registered project. */
export function groupSessionsByWorkspace(
  sessions: ChatSession[],
  doc: WorkspaceDoc | null,
  labelForMissing: string,
): SessionWorkspaceGroup[] {
  const buckets = new Map<string | null, ChatSession[]>();
  for (const session of sessions) {
    const name = sessionWorkspaceName(session, doc);
    const list = buckets.get(name) ?? [];
    list.push(session);
    buckets.set(name, list);
  }
  const named = [...buckets.keys()]
    .filter((k): k is string => k != null)
    .sort((a, b) => a.localeCompare(b));
  const groups: SessionWorkspaceGroup[] = named.map((name) => ({
    name,
    label: name,
    sessions: (buckets.get(name) ?? []).sort((a, b) => b.updatedAt - a.updatedAt),
  }));
  const loose = buckets.get(null);
  if (loose?.length) {
    groups.push({
      name: null,
      label: labelForMissing,
      sessions: loose.sort((a, b) => b.updatedAt - a.updatedAt),
    });
  }
  return groups;
}

/** Cursor-style sidebar: every registered project row, even with zero chats. */
export function listProjectGroupsForSidebar(
  sessions: ChatSession[],
  doc: WorkspaceDoc | null,
  labelForMissing: string,
): SessionWorkspaceGroup[] {
  const fromSessions = groupSessionsByWorkspace(sessions, doc, labelForMissing);
  if (!doc || Object.keys(doc.workspaces).length === 0) {
    return fromSessions;
  }
  const sessionMap = new Map(
    fromSessions.filter((g) => g.name != null).map((g) => [g.name!, g.sessions]),
  );
  const names = orderedProjectNames(doc);
  const groups: SessionWorkspaceGroup[] = names.map((name) => ({
    name,
    label: name,
    sessions: (sessionMap.get(name) ?? []).sort((a, b) => b.updatedAt - a.updatedAt),
  }));
  const loose = fromSessions.find((g) => g.name === null);
  if (loose?.sessions.length) {
    groups.push(loose);
  }
  return groups;
}

export function parentDirectoryForPicker(projectPath: string): string | undefined {
  const normalized = projectPath.trim().replace(/\\/g, "/");
  if (!normalized) return undefined;
  const idx = normalized.lastIndexOf("/");
  if (idx <= 0) return undefined;
  return normalized.slice(0, idx);
}
