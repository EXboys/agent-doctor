import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { openUrl } from "@tauri-apps/plugin-opener";
import { isTeamEdition } from "./edition";
import { t } from "./i18n";

/** China-first download landing; keep in sync with updater CDN + docs. */
const CDN_ROOT = "https://agent-doctor.oss-cn-shenzhen.aliyuncs.com";
export const UPDATE_MANUAL_URL = isTeamEdition()
  ? `${CDN_ROOT}/desktop-team/`
  : `${CDN_ROOT}/desktop/`;
export const UPDATE_GITHUB_URL =
  "https://github.com/EXboys/agent-doctor/releases/latest";

let checking = false;
const appIconUrl = new URL("./app-icon.png", import.meta.url).href;

function showUpdateDialog(opts: {
  title: string;
  body: string;
  ok: string;
  cancel?: string;
}): Promise<boolean> {
  return new Promise((resolve) => {
    const root = document.createElement("div");
    root.className = "update-sheet";
    const card = document.createElement("div");
    card.className = "update-sheet-card";
    card.setAttribute("role", "dialog");
    card.setAttribute("aria-modal", "true");
    card.setAttribute("aria-labelledby", "update-sheet-title");

    const icon = document.createElement("img");
    icon.className = "update-sheet-icon";
    icon.src = appIconUrl;
    icon.alt = "";

    const title = document.createElement("h2");
    title.id = "update-sheet-title";
    title.className = "update-sheet-title";
    title.textContent = opts.title;

    const body = document.createElement("p");
    body.className = "update-sheet-body";
    body.textContent = opts.body;

    const actions = document.createElement("div");
    actions.className = "update-sheet-actions";

    let settled = false;
    const finish = (ok: boolean) => {
      if (settled) return;
      settled = true;
      root.remove();
      document.removeEventListener("keydown", onKey);
      resolve(ok);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") finish(opts.cancel ? false : true);
    };

    const okBtn = document.createElement("button");
    okBtn.type = "button";
    okBtn.className = "btn-primary";
    okBtn.textContent = opts.ok;
    okBtn.addEventListener("click", () => finish(true));

    if (opts.cancel) {
      const cancelBtn = document.createElement("button");
      cancelBtn.type = "button";
      cancelBtn.className = "btn-secondary";
      cancelBtn.textContent = opts.cancel;
      cancelBtn.addEventListener("click", () => finish(false));
      actions.append(cancelBtn, okBtn);
    } else {
      actions.append(okBtn);
    }

    card.append(icon, title, body, actions);
    root.append(card);
    root.addEventListener("click", (event) => {
      if (event.target === root) finish(opts.cancel ? false : true);
    });
    document.addEventListener("keydown", onKey);
    document.body.append(root);
    okBtn.focus();
  });
}

/** True when updater rejects the running binary (tauri:dev / /var symlink path). */
function isDevUpdaterPathError(raw: string): boolean {
  return /StartingBinary|symlink on a non-allowed platform|contains a symlink/i.test(raw);
}

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
        await showUpdateDialog({
          title: t("update.title"),
          body: t("update.upToDate"),
          ok: t("update.ok"),
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

    const shouldInstall = await showUpdateDialog({
      title: t("update.title"),
      body: detail,
      ok: t("update.install"),
      cancel: t("update.later"),
    });
    if (!shouldInstall) return;

    await update.downloadAndInstall();
    const restart = await showUpdateDialog({
      title: t("update.title"),
      body: t("update.restartPrompt"),
      ok: t("update.restart"),
      cancel: t("update.later"),
    });
    if (restart) {
      await relaunch();
    }
  } catch (error) {
    const raw = String(error ?? "");
    const devPath = isDevUpdaterPathError(raw);
    // Dev / unsigned builds have no updater artifacts — keep quiet on boot.
    if (
      silent &&
      (devPath ||
        /not available|unsupported|network|fetch|dns|timed out/i.test(raw))
    ) {
      return;
    }
    if (interactive || !silent) {
      const openManual = await showUpdateDialog({
        title: t("update.title"),
        body: devPath ? t("update.devUnsupported") : t("update.failed", { error: raw.slice(0, 240) }),
        ok: t("update.openManual"),
        cancel: t("update.later"),
      });
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
