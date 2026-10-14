import { SKILL_CATEGORY_ORDER, type SkillCategoryId } from "../skill-categories";

const STORAGE_KEY = "agent-doctor.chat.skillCategories.v1";

export type SkillCategoryOverrides = Record<string, SkillCategoryId>;

export function loadSkillCategoryOverrides(): SkillCategoryOverrides {
  try {
    const parsed = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as Record<string, unknown>;
    const out: SkillCategoryOverrides = {};
    for (const [id, category] of Object.entries(parsed)) {
      if (
        id.trim() &&
        typeof category === "string" &&
        SKILL_CATEGORY_ORDER.includes(category as SkillCategoryId)
      ) {
        out[id] = category as SkillCategoryId;
      }
    }
    return out;
  } catch {
    return {};
  }
}

export function saveSkillCategoryOverride(skillId: string, category: SkillCategoryId): void {
  const id = skillId.trim();
  if (!id) return;
  try {
    const overrides = loadSkillCategoryOverrides();
    overrides[id] = category;
    localStorage.setItem(STORAGE_KEY, JSON.stringify(overrides));
  } catch {
    /* Keep the current visual order when storage is unavailable. */
  }
}
