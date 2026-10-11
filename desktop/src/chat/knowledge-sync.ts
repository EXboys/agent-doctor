import { isPersonalEdition } from "../edition";
import { t } from "../i18n";
import { knowledgeSync, type KnowledgeSyncReport } from "../ipc";

const pending = new Map<string, number>();
/** The folder the open knowledge page is showing. Undefined hides the status line. */
let visiblePath: string | null | undefined;

export function watchKnowledgeSync(projectPath: string | null | undefined): void {
  visiblePath = projectPath;
  if (projectPath === undefined) paint("");
}

function when(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { hour12: false });
}

function lineFor(report: KnowledgeSyncReport): string {
  if (report.state === "idle") return "";
  if (report.state === "synced") {
    return t("chat.knowledgeSyncSynced", {
      time: report.syncedAtMs ? when(report.syncedAtMs) : when(Date.now()),
    });
  }
  if (report.state === "off") return t("chat.knowledgeSyncOff");
  if (report.state === "signed_out") return t("chat.knowledgeSyncSignedOut");
  if (report.state === "too_big") return t("chat.knowledgeSyncTooBig");
  if (report.state === "held") return t("chat.knowledgeSyncHeld");
  const detail = report.detail.trim();
  return detail || t("chat.knowledgeSyncFailed");
}

function paint(text: string): void {
  const el = document.querySelector<HTMLElement>("#chat-knowledge-sync");
  if (!el) return;
  el.textContent = text;
  el.hidden = !text;
}

/** Push and pull one folder. Empty project path is the global knowledge base. */
export function syncKnowledgeFolder(projectPath: string | null): void {
  if (!isPersonalEdition()) return;
  const key = projectPath ?? "";
  const previous = pending.get(key);
  if (previous) window.clearTimeout(previous);
  pending.set(
    key,
    window.setTimeout(() => {
      pending.delete(key);
      void knowledgeSync({ projectPath })
        .then((report) => {
          if (visiblePath === undefined || (visiblePath ?? null) !== (projectPath ?? null)) return;
          paint(lineFor(report));
        })
        .catch(() => {
          if (visiblePath === undefined || (visiblePath ?? null) !== (projectPath ?? null)) return;
          paint(t("chat.knowledgeSyncFailed"));
        });
    }, 800),
  );
}
