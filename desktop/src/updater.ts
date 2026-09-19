import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "./i18n";

/** China-first download landing; keep in sync with updater CDN + docs. */
export const UPDATE_MANUAL_URL =
  "https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop/";
export const UPDATE_GITHUB_URL =
  "https://github.com/EXboys/agent-doctor/releases/latest";

let checking = false;

export async function readAppVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "—";
  }
}

export async function openManualDownload(): Promise<void> {
  try {
    await openUrl(UPDATE_MANUAL_URL);
  } catch {
    try {
      await openUrl(UPDATE_GITHUB_URL);
    } catch {
      // ignore
    }
  }
}

/**
 * Check for desktop updates.
 * @param opts.interactive — show dialogs for "up to date" / errors / install confirm
 * @param opts.silent — used on boot; only prompt when an update exists
 */
export async function checkForAppUpdates(opts?: {
  interactive?: boolean;
  silent?: boolean;
}): Promise<void> {
  const interactive = opts?.interactive ?? true;
  const silent = opts?.silent ?? false;
  if (checking) return;
  checking = true;
  try {
    const update = await check();
    if (!update) {
      if (interactive && !silent) {
        await message(t("update.upToDate"), {
          title: t("update.title"),
          kind: "info",
        });
      }
      return;
    }

    const notes = (update.body ?? "").trim();
    const detail = notes
      ? t("update.availableWithNotes", {
          version: update.version,
          notes: notes.slice(0, 600),
        })
      : t("update.available", { version: update.version });

    const shouldInstall = await ask(detail, {
      title: t("update.title"),
      kind: "info",
      okLabel: t("update.install"),
      cancelLabel: t("update.later"),
    });
    if (!shouldInstall) return;

    await update.downloadAndInstall();
    const restart = await ask(t("update.restartPrompt"), {
      title: t("update.title"),
      kind: "info",
      okLabel: t("update.restart"),
      cancelLabel: t("update.later"),
    });
    if (restart) {
      await relaunch();
    }
  } catch (error) {
    const raw = String(error ?? "");
    // Dev / unsigned builds have no updater artifacts — keep quiet on boot.
    if (silent && /not available|unsupported|network|fetch|dns|timed out/i.test(raw)) {
      return;
    }
    if (interactive || !silent) {
      const openManual = await ask(
        t("update.failed", { error: raw.slice(0, 240) }),
        {
          title: t("update.title"),
          kind: "error",
          okLabel: t("update.openManual"),
          cancelLabel: t("update.later"),
        },
      );
      if (openManual) {
        await openManualDownload();
      }
    }
  } finally {
    checking = false;
  }
}

export async function initUpdaterUi(opts: {
  versionEl: HTMLElement | null;
  checkBtn: HTMLButtonElement | null;
}): Promise<void> {
  if (opts.versionEl) {
    const version = await readAppVersion();
    opts.versionEl.textContent = t("update.version", { version });
  }
  opts.checkBtn?.addEventListener("click", () => {
    void checkForAppUpdates({ interactive: true });
  });
  await listen("check-for-updates", () => {
    void checkForAppUpdates({ interactive: true });
  });
  // Boot check after UI settles; silent unless an update is available.
  window.setTimeout(() => {
    void checkForAppUpdates({ silent: true, interactive: true });
  }, 8_000);
}
