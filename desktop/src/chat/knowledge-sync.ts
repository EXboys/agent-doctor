import { isPersonalEdition } from "../edition";
import { t } from "../i18n";
import { knowledgeSync, type KnowledgeSyncReport } from "../ipc";
import { readKnowledgeSyncEnabled } from "./knowledge-sync-pref";

const pending = new Map<string, number>();
/** The folder the open knowledge page is showing. Undefined hides the status line. */
let visiblePath: string | null | undefined;

type SyncTone = "ok" | "warn" | "bad";

type SyncView = {
  tone: SyncTone;
  label: string;
  /** Shown beside a successful sync. The exact clock time stays on the tooltip. */
  time?: string;
  at?: number;
  detail: string;
  /** Opens the sync switch in Settings. */
  settings?: string;
};

let openSettings: (() => void) | null = null;

/** Settings registers this so the knowledge page can open the sync switch. */
export function bindKnowledgeSyncSettings(open: () => void): void {
  openSettings = open;
}

export function watchKnowledgeSync(projectPath: string | null | undefined): void {
  visiblePath = projectPath;
  if (projectPath === undefined) paint(null);
}

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { hour12: false });
}

function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
}

/** A short time for the chip. The exact timestamp stays in the tooltip. */
function shortWhen(ms: number): string {
  const date = new Date(ms);
  const delta = Date.now() - ms;
  if (delta >= 0 && delta < 45_000) return t("chat.knowledgeSyncJustNow");
  if (delta >= 0 && delta < 60 * 60_000) {
    const minutes = Math.max(1, Math.round(delta / 60_000));
    return t("chat.knowledgeSyncMinutesAgo", { n: String(minutes) });
  }
  const hm = clock(ms);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  if (date.getTime() >= today.getTime()) return hm;
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  if (date.getTime() >= yesterday.getTime()) return t("chat.knowledgeSyncYesterday", { time: hm });
  const md = date.toLocaleDateString(undefined, { month: "numeric", day: "numeric" });
  return `${md} ${hm}`;
}

function localOffView(): SyncView {
  const detail = t("chat.knowledgeSyncLocalOff");
  return {
    tone: "warn",
    label: detail,
    detail,
    settings: t("chat.knowledgeSyncOpenSettings"),
  };
}

function viewFor(report: KnowledgeSyncReport): SyncView | null {
  if (!readKnowledgeSyncEnabled()) return localOffView();
  if (report.state === "idle") return null;
  if (report.state === "synced") {
    const at = report.syncedAtMs ?? Date.now();
    return {
      tone: "ok",
      label: t("chat.knowledgeSyncChipSynced"),
      time: shortWhen(at),
      at,
      detail: t("chat.knowledgeSyncSynced", { time: when(at) }),
      settings: t("chat.knowledgeSyncTurnOff"),
    };
  }
  if (report.state === "off") {
    const detail = t("chat.knowledgeSyncOff");
    return { tone: "warn", label: detail, detail };
  }
  if (report.state === "signed_out") {
    const detail = t("chat.knowledgeSyncSignedOut");
    return { tone: "warn", label: detail, detail };
  }
  if (report.state === "too_big") {
    const detail = t("chat.knowledgeSyncTooBig");
    return { tone: "warn", label: detail, detail };
  }
  if (report.state === "held") {
    const detail = t("chat.knowledgeSyncHeld");
    return { tone: "warn", label: detail, detail };
  }
  const detail = report.detail.trim() || t("chat.knowledgeSyncFailed");
  return { tone: "bad", label: detail, detail };
}

function paint(view: SyncView | null): void {
  const el = document.querySelector<HTMLElement>("#chat-knowledge-sync");
  if (!el) return;
  if (!view) {
    el.hidden = true;
    el.replaceChildren();
    el.removeAttribute("title");
    el.removeAttribute("aria-label");
    delete el.dataset.tone;
    return;
  }
  el.hidden = false;
  el.dataset.tone = view.tone;
  el.title = view.detail;
  el.setAttribute("aria-label", view.detail);
  const dot = document.createElement("span");
  dot.className = "chat-knowledge-sync-dot";
  dot.setAttribute("aria-hidden", "true");
  const label = document.createElement("span");
  label.textContent = view.label;
  if (view.tone === "ok") {
    const pill = document.createElement("span");
    pill.className = "chat-knowledge-sync-pill";
    pill.append(dot, label);
    if (view.time) {
      const time = document.createElement("time");
      if (view.at) time.dateTime = new Date(view.at).toISOString();
      time.textContent = view.time;
      pill.append(time);
    }
    el.replaceChildren(pill);
  } else {
    el.replaceChildren(dot, label);
  }
  if (view.settings) el.append(settingsButton(view.settings));
}

function settingsButton(label: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "chat-knowledge-sync-link";
  button.textContent = label;
  button.addEventListener("click", () => openSettings?.());
  return button;
}

/** Re-read the setting for the knowledge page that is open. */
export function refreshVisibleKnowledgeSync(): void {
  if (visiblePath === undefined) return;
  syncKnowledgeFolder(visiblePath);
}

/** Push and pull one folder. Empty project path is the global knowledge base. */
export function syncKnowledgeFolder(projectPath: string | null): void {
  if (!isPersonalEdition()) return;
  const key = projectPath ?? "";
  const showing = visiblePath !== undefined && (visiblePath ?? null) === (projectPath ?? null);
  if (!readKnowledgeSyncEnabled()) {
    const previous = pending.get(key);
    if (previous) window.clearTimeout(previous);
    pending.delete(key);
    if (showing) paint(localOffView());
    return;
  }
  const previous = pending.get(key);
  if (previous) window.clearTimeout(previous);
  pending.set(
    key,
    window.setTimeout(() => {
      pending.delete(key);
      void knowledgeSync({ projectPath })
        .then((report) => {
          if (visiblePath === undefined || (visiblePath ?? null) !== (projectPath ?? null)) return;
          paint(viewFor(report));
        })
        .catch(() => {
          if (visiblePath === undefined || (visiblePath ?? null) !== (projectPath ?? null)) return;
          paint(viewFor({ state: "failed", syncedAtMs: null, detail: "" }));
        });
    }, 800),
  );
}
