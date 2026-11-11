//! Import existing conversations from external agent CLIs.
//!
//! One source per agent runtime (the registry below is the extension point:
//! a new agent CLI with a local transcript store adds one entry and needs no
//! UI or command changes). Each source lists its store, projects one
//! transcript into chat messages, and turns the payload into a chat session
//! that carries that runtime's resume id — so「继续」resumes the original
//! thread (`claude --resume` / Codex `thread/resume`).

import { t } from "../i18n";
import { importTranscript, listTranscripts } from "../ipc";
import type { AskRuntime } from "../ask-resources";
import type {
  ChatMessage,
  ChatSession,
  TranscriptImportCandidate,
  TranscriptImportPayload,
  TranscriptImportSourceId,
} from "./types";
export type ImportSource = {
  id: TranscriptImportSourceId;
  /** Localized tab label in the dialog. */
  label: () => string;
  list: () => Promise<TranscriptImportCandidate[]>;
  importOne: (path: string) => Promise<TranscriptImportPayload>;
  toSession: (payload: TranscriptImportPayload) => ChatSession;
};

function sessionFromPayload(
  payload: TranscriptImportPayload,
  runtime: AskRuntime,
  fallbackTitle: string,
): ChatSession {
  const messages: ChatMessage[] = payload.messages ?? [];
  if (payload.truncated) {
    messages.push({
      id: `imp-truncated-${crypto.randomUUID()}`,
      role: "meta",
      content: t("chat.importTruncated"),
      at: messages[messages.length - 1]?.at ?? Date.now(),
      log: true,
    });
  }
  const last = messages[messages.length - 1];
  return {
    id: crypto.randomUUID(),
    title: payload.title || fallbackTitle,
    runtime,
    createdAt: messages[0]?.at ?? Date.now(),
    updatedAt: last?.at ?? Date.now(),
    messages,
    // The import carries the real session/thread id, so「继续」resumes
    // the same thread with the runtime's own resume call.
    runtimeThreadId: payload.session_id || null,
  };
}

export const importSources: ImportSource[] = [
  {
    id: "claude-code",
    label: () => t("chat.importSourceClaudeCode"),
    list: () => listTranscripts("claude-code"),
    importOne: (path) => importTranscript("claude-code", path),
    toSession: (payload) =>
      sessionFromPayload(payload, "claude-code", t("chat.importClaudeFallback")),
  },
  {
    id: "codex",
    label: () => t("chat.importSourceCodex"),
    list: () => listTranscripts("codex"),
    importOne: (path) => importTranscript("codex", path),
    toSession: (payload) =>
      sessionFromPayload(payload, "codex", t("chat.importCodexFallback")),
  },
];

export type ImportDeps = {
  /** Insert the imported session and make it the active chat. */
  onImported: (session: ChatSession) => void;
  status: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
};

/** The sidebar entry: a small button that opens the source picker. */
export function transcriptImportEntry(deps: ImportDeps): HTMLElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "chat-import-entry";
  button.textContent = t("chat.importEntry");
  button.addEventListener("click", () => {
    void openTranscriptImportDialog(deps);
  });
  return button;
}

async function openTranscriptImportDialog(deps: ImportDeps): Promise<void> {
  if (document.querySelector(".chat-import-overlay")) return;
  const overlay = document.createElement("div");
  overlay.className = "chat-import-overlay";
  const card = document.createElement("div");
  card.className = "chat-import-card";
  const head = document.createElement("div");
  head.className = "chat-import-head";
  const title = document.createElement("span");
  title.className = "chat-import-title";
  title.textContent = t("chat.importEntry");
  const close = document.createElement("button");
  close.type = "button";
  close.className = "chat-import-close";
  close.setAttribute("aria-label", t("chat.close"));
  close.textContent = "✕";
  close.addEventListener("click", () => overlay.remove());
  head.append(title, close);
  const tabs = document.createElement("div");
  tabs.className = "chat-import-tabs";
  const list = document.createElement("div");
  list.className = "chat-import-list";
  card.append(head, tabs, list);
  overlay.append(card);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) overlay.remove();
  });
  document.body.appendChild(overlay);

  let active = importSources[0];
  const tabButtons = new Map<TranscriptImportSourceId, HTMLButtonElement>();
  for (const source of importSources) {
    const tab = document.createElement("button");
    tab.type = "button";
    tab.className = "chat-import-tab";
    tab.textContent = source.label();
    tab.addEventListener("click", () => {
      active = source;
      for (const [id, button] of tabButtons) {
        button.classList.toggle("is-active", id === source.id);
      }
      void showSource(source);
    });
    tabButtons.set(source.id, tab);
    tabs.append(tab);
  }

  async function showSource(source: ImportSource): Promise<void> {
    list.replaceChildren(t("chat.importLoading"));
    let candidates;
    try {
      candidates = await source.list();
    } catch (error) {
      list.textContent = `${t("chat.importFailed")}：${String(error)}`;
      return;
    }
    if (candidates.length === 0) {
      list.textContent = t("chat.importNone");
      return;
    }
    list.replaceChildren();
    for (const candidate of candidates) {
      const row = document.createElement("button");
      row.type = "button";
      row.className = "chat-import-row";
      const name = document.createElement("span");
      name.className = "chat-import-row-title";
      name.textContent = candidate.title || candidate.session_id;
      const meta = document.createElement("span");
      meta.className = "chat-import-row-meta";
      const when = candidate.updated_at ? new Date(candidate.updated_at).toLocaleString() : "";
      meta.textContent = `${when} · ${Math.max(1, Math.round(candidate.size_bytes / 1024))} KB`;
      row.append(name, meta);
      row.addEventListener("click", () => {
        row.disabled = true;
        deps.status(t("chat.importing"), "muted");
        void source
          .importOne(candidate.path)
          .then((payload) => {
            const session = source.toSession(payload);
            deps.onImported(session);
            deps.status(t("chat.importDone"), "ok");
            overlay.remove();
          })
          .catch((error: unknown) => {
            row.disabled = false;
            deps.status(`${t("chat.importFailed")}：${String(error)}`, "warn");
          });
      });
      list.appendChild(row);
    }
  }

  tabButtons.get(active.id)?.classList.add("is-active");
  await showSource(active);
}
