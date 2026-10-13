const STORAGE_KEY = "agent-doctor.chat.skillOrder.v1";

export function loadSkillOrder(): string[] {
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

export function saveSkillOrder(order: string[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(order));
  } catch {
    /* quota */
  }
}

/** Saved ids stay in front. Skills that are new, or hidden by the current filter, keep their old relative order. */
export function orderBySaved<T extends { skill_id: string }>(items: T[], saved: string[]): T[] {
  const byId = new Map(items.map((item) => [item.skill_id, item]));
  const out: T[] = [];
  const seen = new Set<string>();
  for (const id of saved) {
    const item = byId.get(id);
    if (!item || seen.has(id)) continue;
    out.push(item);
    seen.add(id);
  }
  for (const item of items) {
    if (seen.has(item.skill_id)) continue;
    out.push(item);
    seen.add(item.skill_id);
  }
  return out;
}

/** Move one skill to the front. Other saved ids stay in their order. */
export function pinSkillId(saved: string[], id: string): string[] {
  const trimmed = id.trim();
  if (!trimmed) return saved;
  return [trimmed, ...saved.filter((item) => item !== trimmed)];
}

/**
 * Reorder the skills currently on screen. Ids that are saved but not in this
 * list (another scope, or filtered out by search) keep their place.
 */
export function reorderSkillIds(
  saved: string[],
  visibleIds: string[],
  fromId: string,
  toId: string,
  position: "before" | "after" = "before",
): string[] {
  if (!fromId || fromId === toId) return saved;
  const visibleSet = new Set(visibleIds);
  if (!visibleSet.has(fromId) || !visibleSet.has(toId)) return saved;

  const visibleNow = orderBySaved(
    visibleIds.map((skill_id) => ({ skill_id })),
    saved,
  ).map((item) => item.skill_id);
  const nextVisible = visibleNow.filter((id) => id !== fromId);
  let toIndex = nextVisible.indexOf(toId);
  if (toIndex < 0) return saved;
  if (position === "after") toIndex += 1;
  nextVisible.splice(toIndex, 0, fromId);

  const queue = [...nextVisible];
  const seen = new Set<string>();
  const out: string[] = [];
  const backbone = [...saved];
  for (const id of visibleNow) {
    if (!backbone.includes(id)) backbone.push(id);
  }
  for (const id of backbone) {
    if (!visibleSet.has(id)) {
      if (seen.has(id)) continue;
      out.push(id);
      seen.add(id);
      continue;
    }
    while (queue.length > 0 && seen.has(queue[0]!)) queue.shift();
    const next = queue.shift();
    if (!next || seen.has(next)) continue;
    out.push(next);
    seen.add(next);
  }
  for (const id of queue) {
    if (seen.has(id)) continue;
    out.push(id);
    seen.add(id);
  }
  return out;
}
