import { openUrl } from "@tauri-apps/plugin-opener";
import { computerUseAccess, computerUsePrompt } from "../ipc";

/** Opens the Accessibility list. The system still needs the person to flip that switch. */
export const MAC_ACCESSIBILITY =
  "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Accessibility";

export function isMac(): boolean {
  return /Mac/i.test(navigator.platform) || /Mac OS/i.test(navigator.userAgent);
}

/** System dialog only when this app is not already allowed. */
export async function openMacAccessibility(): Promise<void> {
  const access = await computerUseAccess().catch(() => null);
  if (access?.trusted) return;
  if (access?.step === "screen" && access.settings_url) {
    await openUrl(access.settings_url);
    return;
  }
  const prompted = computerUsePrompt()
    .then(() => true)
    .catch(() => false);
  const opened = openUrl(MAC_ACCESSIBILITY)
    .then(() => true)
    .catch(() => false);
  const [didPrompt, didOpen] = await Promise.all([prompted, opened]);
  if (!didPrompt && !didOpen) throw new Error("accessibility guide did not open");
}
