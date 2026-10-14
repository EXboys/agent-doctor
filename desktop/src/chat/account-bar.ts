import { openUrl } from "@tauri-apps/plugin-opener";
import { t } from "../i18n";
import { teamupsAccountStatus } from "../ipc";
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

export function bindAccountBar(): void {
  const account = document.querySelector<HTMLButtonElement>("#chat-account");
  const update = document.querySelector<HTMLButtonElement>("#chat-update");
  account?.addEventListener("click", () => {
    void openUrl(teamupsAccountPageUrl()).catch(() => {
      // Outside the app there is no system browser hook.
    });
  });
  update?.addEventListener("click", () => {
    void checkForAppUpdates({ interactive: true });
  });
  syncAccountBarLabels();
  void teamupsAccountStatus()
    .then((status) => {
      setTeamupsBaseUrl(status.base_url);
      const name = status.signed_in ? status.name?.trim() || "" : "";
      paintAccount(name, status.signed_in);
    })
    .catch(() => {
      paintAccount("", false);
    });
  window.setTimeout(() => {
    void probeAppUpdate().then((version) => {
      if (!version || !update) return;
      update.hidden = false;
      update.title = t("chat.updateReadyHint");
    });
  }, 8_000);
}
