const KEY = "agent-doctor-knowledge-sync";

/** Knowledge follows the account unless this personal setting is off. */
export function readKnowledgeSyncEnabled(): boolean {
  try {
    return localStorage.getItem(KEY) !== "off";
  } catch {
    return true;
  }
}

export function writeKnowledgeSyncEnabled(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    /* ignore */
  }
}
