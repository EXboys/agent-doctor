import { t } from "../i18n";
import {
  backupStoreRaw,
  hasRestorableChatBackup,
  parseStoreRaw,
} from "./store";
import {
  LEGACY_STORAGE_KEY,
  STORAGE_BACKUP_KEY,
  STORAGE_KEY,
  type ChatSession,
  type SessionStore,
} from "./types";

export type BackupUiDeps = {
  logEl: HTMLElement;
  titleEl: HTMLElement;
  getStore: () => SessionStore;
  setStore: (store: SessionStore) => void;
  activeSession: () => ChatSession;
  sessionTitle: (session: ChatSession) => string;
  flushStorePersist: () => void;
  renderActiveMessages: () => void;
  renderSessionList: () => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  syncRestoreBackupButton: () => void;
  appendBubble: (
    kind: "assistant" | "user" | "meta" | "permission",
    text: string,
    opts?: { id?: string; persist?: boolean },
  ) => HTMLElement;
};

export type BackupUiApi = ReturnType<typeof createBackupUiController>;

export function createBackupUiController(deps: BackupUiDeps) {
  function restoreChatFromBackup(): boolean {
    const raw = localStorage.getItem(STORAGE_BACKUP_KEY);
    if (!raw) {
      deps.setStatus(t("chat.restoreBackupNone"), "warn");
      return false;
    }
    const loaded = parseStoreRaw(raw);
    if (!loaded) {
      deps.setStatus(t("chat.restoreBackupNone"), "warn");
      return false;
    }
    deps.setStore(loaded);
    deps.flushStorePersist();
    deps.renderActiveMessages();
    deps.renderSessionList();
    deps.titleEl.textContent = deps.sessionTitle(deps.activeSession());
    deps.setStatus(t("chat.restoreBackupOk"), "ok");
    deps.syncRestoreBackupButton();
    return true;
  }

  function offerBackupRestoreIfNeeded(): void {
    if (!hasRestorableChatBackup()) return;
    const session = deps.activeSession();
    const looksEmpty =
      session.messages.length === 0 &&
      deps.getStore().sessions.length <= 1 &&
      !session.title.trim();
    if (!looksEmpty) return;
    deps.appendBubble("meta", t("chat.restoreBackupHint"), { persist: false });
    const row = document.createElement("div");
    row.className = "chat-restore-backup-row";
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "btn-secondary btn-compact";
    btn.textContent = t("chat.restoreBackup");
    btn.addEventListener("click", () => {
      restoreChatFromBackup();
      row.remove();
    });
    row.appendChild(btn);
    deps.logEl.appendChild(row);
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
  }

  function showBootFailure(error: unknown): void {
    console.error("Ask: boot failed", error);
    const shell = document.querySelector<HTMLElement>("#chat-shell");
    if (shell) shell.style.display = "grid";
    const log = document.querySelector<HTMLElement>("#chat-log");
    if (log) {
      log.replaceChildren();
      const box = document.createElement("div");
      box.className = "chat-bubble meta";
      box.style.margin = "12px";
      box.style.lineHeight = "1.5";
      box.textContent =
        "对话页加载失败。多半是本地聊天记录损坏或过大。你可以点下方按钮清空缓存后重试；若仍白屏，请完全退出 Agent Doctor 再打开。";
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "btn-primary";
      btn.style.marginTop = "10px";
      btn.textContent = "清空对话缓存并重试";
      btn.addEventListener("click", () => {
        const current = localStorage.getItem(STORAGE_KEY);
        if (current) backupStoreRaw(current);
        localStorage.removeItem(STORAGE_KEY);
        localStorage.removeItem(LEGACY_STORAGE_KEY);
        window.location.reload();
      });
      const restore = document.createElement("button");
      restore.type = "button";
      restore.className = "btn-secondary";
      restore.style.marginTop = "8px";
      restore.style.marginLeft = "8px";
      restore.textContent = t("chat.restoreBackup");
      restore.addEventListener("click", () => {
        if (restoreChatFromBackup()) {
          box.textContent = t("chat.restoreBackupOk");
          btn.remove();
          restore.remove();
        }
      });
      log.append(box, btn, restore);
    }
    const list = document.querySelector<HTMLElement>("#chat-sessions");
    if (list && list.childElementCount === 0) {
      const hint = document.createElement("p");
      hint.style.margin = "8px 10px";
      hint.style.fontSize = "0.72rem";
      hint.style.color = "var(--muted)";
      hint.textContent = "会话列表暂不可用";
      list.appendChild(hint);
    }
  }

  return {
    restoreChatFromBackup,
    offerBackupRestoreIfNeeded,
    showBootFailure,
  };
}
