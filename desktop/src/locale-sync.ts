import { listen } from "@tauri-apps/api/event";
import { getLocale, setLocale, type Locale } from "./i18n";
import { getAppLocale, setAppLocale } from "./ipc";

function isLocale(value: string | null | undefined): value is Locale {
  return value === "en" || value === "zh";
}

let revision = 0;

/** Save the language once, then tell every open window. */
export async function publishLocale(next: Locale): Promise<void> {
  revision += 1;
  await setLocale(next);
  await setAppLocale(next).catch(() => {});
}

/** Follow the shared language, including a window that opens later. */
export function bindLocaleSync(apply: () => void): void {
  void listen<string>("locale-changed", (event) => {
    if (!isLocale(event.payload) || event.payload === getLocale()) return;
    revision += 1;
    void setLocale(event.payload).then(apply);
  });
  const ticket = revision;
  void (async () => {
    let saved: string | null = null;
    try {
      saved = await getAppLocale();
    } catch {
      return;
    }
    if (ticket !== revision) return;
    if (isLocale(saved)) {
      if (saved !== getLocale()) {
        void setLocale(saved).then(apply);
      }
      return;
    }
    await setAppLocale(getLocale()).catch(() => {});
  })();
}
