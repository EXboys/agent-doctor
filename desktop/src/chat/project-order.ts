import type { WorkspaceDoc } from "../ask-resources";

const STORAGE_KEY = "agent-doctor.chat.projectOrder.v1";

/** Put default / active project first (agent-doctor preferred when tied). */
export function sortRegisteredProjectNames(doc: WorkspaceDoc): string[] {
  const names = Object.keys(doc.workspaces);
  const active = doc.active?.trim();
  return names.sort((a, b) => {
    if (active && a === active) return -1;
    if (active && b === active) return 1;
    if (a === "agent-doctor") return -1;
    if (b === "agent-doctor") return 1;
    return a.localeCompare(b);
  });
}

export function loadProjectOrder(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is string => typeof item === "string" && item.trim().length > 0);
  } catch {
    return [];
  }
}

export function saveProjectOrder(order: string[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(order));
  } catch {
    /* quota */
  }
}

/** Saved drag order wins; unknown projects append; removed names drop out. */
export function mergeProjectOrder(doc: WorkspaceDoc, saved: string[]): string[] {
  const all = new Set(Object.keys(doc.workspaces));
  const out: string[] = [];
  for (const name of saved) {
    if (all.has(name) && !out.includes(name)) out.push(name);
  }
  for (const name of sortRegisteredProjectNames(doc)) {
    if (!out.includes(name)) out.push(name);
  }
  return out;
}

export function orderedProjectNames(doc: WorkspaceDoc): string[] {
  const saved = loadProjectOrder();
  if (saved.length > 0) {
    return mergeProjectOrder(doc, saved);
  }
  return sortRegisteredProjectNames(doc);
}

export function moveProjectInOrder(order: string[], name: string, delta: -1 | 1): string[] {
  const index = order.indexOf(name);
  if (index < 0) return order;
  const target = index + delta;
  if (target < 0 || target >= order.length) return order;
  const next = [...order];
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

export function reorderProjectKeys(
  order: string[],
  fromKey: string,
  toKey: string,
  position: "before" | "after" = "before",
): string[] {
  if (fromKey === toKey) return order;
  const next = order.filter((n) => n !== fromKey);
  let toIndex = next.indexOf(toKey);
  if (toIndex < 0) {
    next.push(fromKey);
    return next;
  }
  if (position === "after") toIndex += 1;
  next.splice(toIndex, 0, fromKey);
  return next;
}
