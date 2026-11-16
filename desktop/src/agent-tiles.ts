/**
 * Which agent tiles the home paints.
 *
 * The built-in doctor counts as an agent: a machine with a single CLI still
 * has two agents to choose from, so the doctor tile has to survive the solo
 * layout that collapses the CLI list.
 */

/** Agent tiles on the home when collapsed, the built-in doctor included (3×2 grid). */
export const HOME_AGENT_VISIBLE = 6;

/** Built-in doctor agent runtime id (mirrors agent-doctor-repair). */
export const DOCTOR_RUNTIME_ID = "doctor";

export type AgentTilePlan = {
  /** How many installed CLI tiles to paint, in scan order. */
  visibleCount: number;
  /** Which CLI runtime ids to paint (keeps a hidden selection in the last slot when collapsed). */
  visibleIds: string[];
  /** The built-in doctor tile is painted in front of the CLI tiles. */
  doctor: boolean;
  /** The "more / show less" overflow link is shown. */
  overflow: boolean;
  /** The selected CLI sits past the collapsed window. */
  expandForSelection: boolean;
  /** How many CLI tiles the overflow link hides. */
  hiddenCount: number;
};

/**
 * Plan the home's agent tiles for one paint.
 *
 * `installedIds` is the installed CLI runtimes in display order, `selectedId`
 * the agent the home currently shows, `expanded` whether the user opened the
 * overflow list, `allowCollapseSelected` whether the user explicitly collapsed
 * the list while a deep agent is selected (keep that selection in the last
 * visible slot so the Collapse button always works).
 */
export function planAgentTiles(
  installedIds: string[],
  selectedId: string,
  expanded: boolean,
  allowCollapseSelected = false,
): AgentTilePlan {
  if (installedIds.length < 2) {
    // Solo machine: one CLI, but the built-in doctor is a second agent.
    return {
      visibleCount: installedIds.length,
      visibleIds: [...installedIds],
      doctor: true,
      overflow: false,
      expandForSelection: false,
      hiddenCount: 0,
    };
  }
  // The doctor tile takes the first slot, so the CLI budget is one smaller.
  const budget = HOME_AGENT_VISIBLE - 1;
  const overflow = installedIds.length > budget;
  const selectedPastBudget =
    overflow &&
    selectedId !== DOCTOR_RUNTIME_ID &&
    installedIds.slice(budget).includes(selectedId);
  const expandForSelection = selectedPastBudget && !expanded && !allowCollapseSelected;
  const showAll = expandForSelection || (overflow && expanded);
  if (showAll) {
    return {
      visibleCount: installedIds.length,
      visibleIds: [...installedIds],
      doctor: true,
      overflow,
      expandForSelection,
      hiddenCount: 0,
    };
  }
  const count = Math.min(installedIds.length, budget);
  const visibleIds = installedIds.slice(0, count);
  if (selectedPastBudget && count > 0) {
    visibleIds[count - 1] = selectedId;
  }
  return {
    visibleCount: count,
    visibleIds,
    doctor: true,
    overflow,
    expandForSelection: false,
    hiddenCount: Math.max(0, installedIds.length - count),
  };
}
