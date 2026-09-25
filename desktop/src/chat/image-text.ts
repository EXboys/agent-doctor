const STORAGE_KEY = "ad.ask.readImageText";

/** Default on: text-only models can use picture words; users can turn off to keep chats shorter. */
export function readImageTextEnabled(): boolean {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw == null || raw === "") return true;
    return raw !== "0" && raw !== "false";
  } catch {
    return true;
  }
}

export function setReadImageTextEnabled(on: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, on ? "1" : "0");
  } catch {
    /* ignore */
  }
}
