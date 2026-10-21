import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { teamupsLoginFailure } from "../friendly-error";
import { t } from "../i18n";
import { startTeamupsLogin, teamupsAccountStatus } from "../ipc";
import {
  setTeamupsBaseUrl,
  teamupsAccountPageUrl,
} from "../teamups-site";
import { checkForAppUpdates, probeAppUpdate } from "../updater";

function avatarLetter(name: string): string {
  const trimmed = name.trim();
  const first = [...trimmed][0];
  return first ? first.toUpperCase() : "个";
}

function paintAccount(signedInName: string, signedIn = false): void {
  const name = signedInName.trim();
  const label = name || (signedIn ? t("chat.account") : t("personal.officialLogin"));
  const nameEl = document.querySelector<HTMLElement>("#chat-account-name");
  const avatarEl = document.querySelector<HTMLElement>("#chat-account-avatar");
  const button = document.querySelector<HTMLButtonElement>("#chat-account");
  if (nameEl) {
    nameEl.textContent = label;
    if (signedIn) nameEl.dataset.signedIn = "1";
    else delete nameEl.dataset.signedIn;
    if (name) nameEl.dataset.accountName = name;
    else delete nameEl.dataset.accountName;
  }
  if (avatarEl) avatarEl.textContent = avatarLetter(name || label);
  if (button) button.title = signedIn ? t("chat.accountHint") : t("personal.officialLogin");
}

export function syncAccountBarLabels(): void {
  const update = document.querySelector<HTMLButtonElement>("#chat-update");
  const settings = document.querySelector<HTMLButtonElement>("#chat-settings-open");
  const nameEl = document.querySelector<HTMLElement>("#chat-account-name");
  if (update) {
    update.textContent = t("chat.updateReady");
    update.title = t("chat.updateReadyHint");
  }
  if (settings) {
    settings.title = t("chat.settings");
    settings.setAttribute("aria-label", t("chat.settings"));
  }
  paintAccount(nameEl?.dataset.accountName || "", nameEl?.dataset.signedIn === "1");
}

let accountSignedIn = false;
let accountLoginWaiting = false;

function showAccountLoggingIn(): void {
  const nameEl = document.querySelector<HTMLElement>("#chat-account-name");
  const button = document.querySelector<HTMLButtonElement>("#chat-account");
  if (nameEl) {
    nameEl.textContent = t("chat.accountLoggingIn");
    delete nameEl.dataset.signedIn;
  }
  if (button) button.title = t("resources.mallLoggingIn");
}

function showAccountProblem(message: string): void {
  const nameEl = document.querySelector<HTMLElement>("#chat-account-name");
  const button = document.querySelector<HTMLButtonElement>("#chat-account");
  if (nameEl) {
    nameEl.textContent = message;
    delete nameEl.dataset.signedIn;
    delete nameEl.dataset.accountName;
  }
  if (button) button.title = message;
}

async function refreshAccountBar(): Promise<void> {
  try {
    const status = await teamupsAccountStatus();
    setTeamupsBaseUrl(status.base_url);
    accountSignedIn = status.signed_in;
    if (status.signed_in) accountLoginWaiting = false;
    if (accountLoginWaiting) return;
    const name = status.signed_in ? status.name?.trim() || "" : "";
    paintAccount(name, status.signed_in);
  } catch {
    if (accountLoginWaiting) return;
    accountSignedIn = false;
    paintAccount("", false);
  }
}

export function bindAccountBar(): void {
  const account = document.querySelector<HTMLButtonElement>("#chat-account");
  const update = document.querySelector<HTMLButtonElement>("#chat-update");
  account?.addEventListener("click", () => {
    if (accountSignedIn) {
      void openUrl(teamupsAccountPageUrl()).catch(() => {
        // Outside the app there is no system browser hook.
      });
      return;
    }
    if (accountLoginWaiting) return;
    accountLoginWaiting = true;
    showAccountLoggingIn();
    void startTeamupsLogin()
      .then((started) => openUrl(started.verification_url))
      .catch((error) => {
        accountLoginWaiting = false;
        showAccountProblem(teamupsLoginFailure(error));
      });
  });
  update?.addEventListener("click", () => {
    void checkForAppUpdates({ interactive: true });
  });
  syncAccountBarLabels();
  void refreshAccountBar();
  void listen("teamups-account-changed", () => {
    void refreshAccountBar();
  });
  void listen<{ status?: string; message?: string }>("teamups-login-finished", (event) => {
    if (!accountLoginWaiting) return;
    const status = event.payload?.status || "";
    if (status === "approved") {
      accountLoginWaiting = false;
      void refreshAccountBar();
      return;
    }
    accountLoginWaiting = false;
    const message =
      status === "expired" || status === "denied"
        ? t("resources.mallLoginExpired")
        : teamupsLoginFailure(event.payload?.message || "");
    showAccountProblem(message);
  });
  window.setTimeout(() => {
    void probeAppUpdate().then((version) => {
      if (!version || !update) return;
      update.hidden = false;
      update.title = t("chat.updateReadyHint");
    });
  }, 8_000);
}
