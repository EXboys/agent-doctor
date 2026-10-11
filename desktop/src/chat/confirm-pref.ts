const KEY = "agent-doctor-settings-confirm";

/** New chats start with auto-approve off, unless this personal setting is on. */
export function readAutoApproveDefault(): boolean {
  try {
    const saved = localStorage.getItem(KEY);
    return saved === "on" || saved === "auto";
  } catch {
    return false;
  }
}

export function writeAutoApproveDefault(on: boolean): void {
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    /* ignore */
  }
}
