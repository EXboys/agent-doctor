/**
 * Which agent tiles the home paints.
 *
 * The built-in doctor counts as an agent: a machine with a single CLI still
 * has two agents to choose from, so the doctor tile has to survive the solo
 * layout that collapses the CLI list.
 */

/** Agent tiles on the home, the built-in doctor included. */
export const HOME_AGENT_VISIBLE = 6;

/** Built-in doctor agent runtime id (mirrors agent-doctor-repair). */
export const DOCTOR_RUNTIME_ID = "doctor";

export type AgentTilePlan = {
  /** How many installed CLI tiles to paint, in scan order. */
  visibleCount: number;
  /** The built-in doctor tile is painted in front of the CLI tiles. */
  doctor: boolean;
  /** The "more" overflow link is shown. */
  overflow: boolean;
  /** The selected CLI sits past the visible window — expand to reveal it. */
  expandForSelection: boolean;
  /** How many CLI tiles the overflow link hides. */
  hiddenCount: number;
};

/**
 * Plan the home's agent tiles for one paint.
 *
 * `installedIds` is the installed CLI runtimes in scan order, `selectedId` the
 * agent the home currently shows, `expanded` whether the user opened the
 * overflow list.
 */
export function planAgentTiles(
  installedIds: string[],
  selectedId: string,
  expanded: boolean,
): AgentTilePlan {
  if (installedIds.length < 2) {
    // Solo machine: one CLI, but the built-in doctor is a second agent.
    return {
      visibleCount: installedIds.length,
      doctor: true,
      overflow: false,
      expandForSelection: false,
      hiddenCount: 0,
    };
  }
  // The doctor tile takes the first slot, so the CLI budget is one smaller.
  const budget = HOME_AGENT_VISIBLE - 1;
  const overflow = installedIds.length > budget;
  const expandForSelection =
    overflow &&
    selectedId !== DOCTOR_RUNTIME_ID &&
    installedIds.slice(budget).includes(selectedId);
  const showAll = expandForSelection || (overflow && expanded);
  return {
    visibleCount: showAll ? installedIds.length : Math.min(installedIds.length, budget),
    doctor: true,
    overflow,
    expandForSelection,
    hiddenCount: showAll ? 0 : Math.max(0, installedIds.length - budget),
  };
}
