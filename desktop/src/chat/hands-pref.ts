const BROWSER_KEY = "agent-doctor-browser-tools";
const COMPUTER_KEY = "agent-doctor-computer-use";

function readFlag(key: string): boolean {
  try {
    return localStorage.getItem(key) === "on";
  } catch {
    return false;
  }
}

function writeFlag(key: string, on: boolean): void {
  try {
    localStorage.setItem(key, on ? "on" : "off");
  } catch {
    /* ignore */
  }
}

/** Opening web pages, clicking, and filling. Off until the person turns it on. */
export function readBrowserToolsEnabled(): boolean {
  return readFlag(BROWSER_KEY);
}

export function writeBrowserToolsEnabled(on: boolean): void {
  writeFlag(BROWSER_KEY, on);
}

/** Clicking other apps on this computer. Off until the person turns it on. */
export function readComputerUseEnabled(): boolean {
  return readFlag(COMPUTER_KEY);
}

export function writeComputerUseEnabled(on: boolean): void {
  writeFlag(COMPUTER_KEY, on);
}
