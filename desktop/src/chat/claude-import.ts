import { invoke } from "@tauri-apps/api/core";
import { t } from "../i18n";
import type { ChatMessage, ChatSession } from "./types";

export type ClaudeImportCandidate = {
  path: string;
  session_id: string;
  title: string;
  preview: string;
  updated_at: number;
  size_bytes: number;
};

type ClaudeImportPayload = {
  session_id: string;
  title: string;
  truncated: boolean;
  messages: ChatMessage[];
};

export type ClaudeImportDeps = {
  /** Insert the imported session and make it the active chat. */
  onImported: (session: ChatSession) => void;
  status: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
};

function listCandidates(): Promise<ClaudeImportCandidate[]> {
  return invoke("list_claude_import_sessions_command");
}

function importPayload(path: string): Promise<ClaudeImportPayload> {
  return invoke("import_claude_session_command", { path });
}

export function importedSessionFromPayload(payload: ClaudeImportPayload): ChatSession {
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
    title: payload.title || t("chat.importClaude"),
    runtime: "claude-code",
    createdAt: messages[0]?.at ?? Date.now(),
    updatedAt: last?.at ?? Date.now(),
    messages,
    // The import carries the real Claude session id, so "continue" resumes
    // the same thread with `claude --resume`.
    runtimeThreadId: payload.session_id || null,
  };
}

/** The sidebar entry: a small button that opens the candidate picker. */
export function claudeImportEntry(deps: ClaudeImportDeps): HTMLElement {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "chat-import-claude";
  button.textContent = t("chat.importClaude");
  button.addEventListener("click", () => {
    void openClaudeImportDialog(deps);
  });
  return button;
}

async function openClaudeImportDialog(deps: ClaudeImportDeps): Promise<void> {
  if (document.querySelector(".chat-import-overlay")) return;
  const overlay = document.createElement("div");
  overlay.className = "chat-import-overlay";
  const card = document.createElement("div");
  card.className = "chat-import-card";
  const head = document.createElement("div");
  head.className = "chat-import-head";
  const title = document.createElement("span");
  title.className = "chat-import-title";
  title.textContent = t("chat.importClaude");
  const close = document.createElement("button");
  close.type = "button";
  close.className = "chat-import-close";
  close.setAttribute("aria-label", t("chat.close"));
  close.textContent = "✕";
  close.addEventListener("click", () => overlay.remove());
  head.append(title, close);
  const list = document.createElement("div");
  list.className = "chat-import-list";
  list.textContent = t("chat.importLoading");
  card.append(head, list);
  overlay.append(card);
  overlay.addEventListener("click", (event) => {
    if (event.target === overlay) overlay.remove();
  });
  document.body.appendChild(overlay);

  let candidates: ClaudeImportCandidate[] = [];
  try {
    candidates = await listCandidates();
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
      void importPayload(candidate.path)
        .then((payload) => {
          const session = importedSessionFromPayload(payload);
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
