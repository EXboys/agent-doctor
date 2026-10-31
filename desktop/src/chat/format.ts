import type { AskRuntime } from "../ask-resources";
import { explainChatFailure, formatChatFailureLine } from "../friendly-error";
import { t, type MessageKey } from "../i18n";
import type { ChatMessage, ToolStep } from "./types";
import { runtimeDisplayName } from "./runtime";

const LOG_COLOR = /\u001b\[[0-9;]*[A-Za-z]|\[[0-9;]+m/g;

/** Drop terminal color codes, including leftovers after the ESC byte is gone. */
export function stripLogAnsi(raw: string): string {
  return raw.replace(LOG_COLOR, "");
}

/** A runtime log record, recognized by shape rather than by the error text. */
export function isRuntimeLog(raw: string): boolean {
  if (/\u001b\[[0-9;]*[A-Za-z]|\[[0-9;]+m/.test(raw)) return true;
  const text = stripLogAnsi(raw).trim();
  if (SUBSYSTEM_TAG.test(text)) return true;
  return /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}/.test(text) && /\b(ERROR|WARN|WARNING|INFO|DEBUG)\b/.test(text);
}

/** An HTTP response body pasted into a status line. That is the original, not the sentence. */
export function isTransportDump(raw: string): boolean {
  return /HTTP\s+\d{3}\b/.test(raw) && raw.includes("{");
}

/** `[skills] …` / `[state-migrations] …`: a lowercase subsystem tag opening the line. */
const SUBSYSTEM_TAG = /^\[[a-z][a-z0-9_./:-]*\]\s/;

const COMMAND_TAIL = /`([\s\S]+)`\s*$/;

/** Original text worth keeping behind a collapsed row. */
export function failedCommandScript(raw: string): string | null {
  if (!isRuntimeLog(raw)) return null;
  const text = stripLogAnsi(raw).trim();
  const quoted = text.match(COMMAND_TAIL);
  return quoted?.[1]?.trim() || text;
}

/** Short line, with the original script collapsed. Only for a log that quotes a command. */
export function mountFailedCommand(parent: HTMLElement, raw: string): boolean {
  const script = failedCommandScript(raw);
  if (!script || !COMMAND_TAIL.test(stripLogAnsi(raw).trim())) return false;
  parent.replaceChildren();
  const summary = document.createElement("div");
  summary.className = "chat-activity-text";
  summary.textContent = t("chat.commandFailed");
  const more = document.createElement("details");
  more.className = "chat-permission-more";
  const moreSummary = document.createElement("summary");
  moreSummary.textContent = t("chat.permissionExpand");
  const detail = document.createElement("pre");
  detail.className = "chat-permission-detail";
  detail.textContent = script;
  more.append(moreSummary, detail);
  parent.append(summary, more);
  return true;
}

/** A saved message that holds raw process output. Older saves only have the shape to go by. */
export function isLogMessage(message: { role: string; content: string; log?: boolean }): boolean {
  if (message.role !== "meta") return false;
  if (message.log) return true;
  return isRuntimeLog(message.content) && !COMMAND_TAIL.test(stripLogAnsi(message.content).trim());
}

const RUN_LOG_CHARS = 20_000;

/** This turn's collapsed log row: after the last user message, if there is one yet. */
function currentRunLog(logEl: HTMLElement): HTMLElement | null {
  for (let el = logEl.lastElementChild as HTMLElement | null; el; el = el.previousElementSibling as HTMLElement | null) {
    if (el.classList.contains("chat-msg-user")) return null;
    if (el.dataset.runtimeLog === "1") return el;
  }
  return null;
}

const OLD_NO_REPLY = new Set([
  "这次没有回话。运行记录收在上面，点开可以看原文。再发一次试试。",
  "No reply this time. The run notes are above; open them to see the original text. Try sending again.",
]);

/** Saved copies of the old “open the original above” line. That line is not shown again. */
export function isRunLogPointer(text: string): boolean {
  return OLD_NO_REPLY.has(text.trim());
}

/** What to say when the agent failed and wrote no reply. */
export function agentFailedLine(runtime: AskRuntime): string {
  return t("chat.agentFailed", { name: runtimeDisplayName(runtime) });
}

/**
 * Sentence on a failed run-log row. A log that already says why (a limit, an
 * expired sign-in, a refused model) uses that sentence. Anything else stays
 * the generic “it reported an error”.
 */
export function runLogFailureLabel(content: string, runtime: AskRuntime): string {
  const explained = explainChatFailure(content);
  if (explained) return formatChatFailureLine(explained);
  return agentFailedLine(runtime);
}

/**
 * A failed turn that only left a log: the row carries the sentence, and the
 * old "notes are above" line is not drawn again.
 */
export function coveredRunLogPointers(
  messages: { role: string; content: string; log?: boolean; summary?: ChatMessage["summary"] }[],
): { failedLogs: Set<number>; hidden: Set<number> } {
  const failedLogs = new Set<number>();
  const hidden = new Set<number>();
  let logAt = -1;
  let hadAssistant = false;
  const finish = () => {
    if (logAt >= 0 && !hadAssistant) failedLogs.add(logAt);
  };
  for (let i = 0; i < messages.length; i += 1) {
    const message = messages[i];
    if (message.role === "user") {
      finish();
      logAt = -1;
      hadAssistant = false;
      continue;
    }
    if (message.role === "assistant" && message.content.trim()) hadAssistant = true;
    if (isLogMessage(message)) {
      logAt = i;
      if (message.summary === "commandFailed") failedLogs.add(i);
      continue;
    }
    if (logAt >= 0 && message.role === "meta" && isRunLogPointer(message.content)) {
      failedLogs.add(logAt);
      hidden.add(i);
    }
  }
  finish();
  return { failedLogs, hidden };
}

/** The log of this turn failed and never became a reply. Returns whether one was found. */
export function markTurnLogFailed(messages: ChatMessage[]): boolean {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    if (message?.role === "user") return false;
    if (message?.role === "meta" && message.log) {
      message.summary = "commandFailed";
      return true;
    }
  }
  return false;
}

/** Point the open turn's log row at the failure sentence. */
export function labelCurrentRunLog(logEl: HTMLElement, summaryText: string): HTMLElement | null {
  const row = currentRunLog(logEl);
  const label = row?.querySelector(".chat-activity-text");
  if (!row || !label) return null;
  label.textContent = summaryText;
  return row;
}

/** Add raw output to this turn's collapsed log, starting one at the end if needed. */
export function appendRunLog(logEl: HTMLElement, raw: string, summaryText = t("chat.runtimeLog")): HTMLElement | null {
  const text = stripLogAnsi(raw).trim();
  if (!text) return null;
  const existing = currentRunLog(logEl);
  const detail = existing?.querySelector<HTMLElement>(".chat-permission-detail");
  if (existing && detail) {
    const next = `${detail.textContent ?? ""}\n${text}`;
    detail.textContent = next.length > RUN_LOG_CHARS ? next.slice(next.length - RUN_LOG_CHARS) : next;
    const label = existing.querySelector(".chat-activity-text");
    if (label && summaryText !== t("chat.runtimeLog")) label.textContent = summaryText;
    return existing;
  }
  const row = document.createElement("div");
  row.className = "chat-activity kind-log";
  row.dataset.kind = "log";
  row.dataset.stderr = "1";
  row.dataset.runtimeLog = "1";
  const summary = document.createElement("div");
  summary.className = "chat-activity-text";
  summary.textContent = summaryText;
  const more = document.createElement("details");
  more.className = "chat-permission-more";
  const moreSummary = document.createElement("summary");
  moreSummary.textContent = t("chat.runtimeLogExpand");
  const pre = document.createElement("pre");
  pre.className = "chat-permission-detail";
  pre.textContent = text;
  more.append(moreSummary, pre);
  row.append(summary, more);
  logEl.appendChild(row);
  return row;
}

/** One sentence for a command the user should allow without reading a shell line. */
export function plainPermissionSummary(raw: string): string | null {
  const lower = raw.toLowerCase();
  if (
    lower.includes("rm -f") ||
    lower.includes("rm -rf") ||
    lower.includes("rm -fr") ||
    lower.includes("safer approach") ||
    /\brm\b[^\n]{0,40}-[a-z]*f/.test(lower)
  ) {
    return t("chat.permissionTempFiles");
  }
  if (
    lower.includes("ffmpeg") ||
    lower.includes("剪视频") ||
    /\.(mp4|mov|mkv|m4v|avi)\b/.test(lower)
  ) {
    return t("chat.permissionVideo");
  }
  return null;
}

/** Prefer a short human summary; keep the full command for the expandable row. */
export function formatPermissionDetail(raw: string): { summary: string; full: string } {
  const full = raw.trim();
  if (!full) {
    return { summary: "", full: "" };
  }
  const plain = plainPermissionSummary(full);
  if (plain) {
    return { summary: plain, full };
  }
  const unfenced = full
    .replace(/^```(?:json)?\s*/i, "")
    .replace(/\s*```$/i, "")
    .trim();
  if (unfenced.startsWith("{") && unfenced.endsWith("}")) {
    try {
      const value = JSON.parse(unfenced) as Record<string, unknown>;
      const description =
        typeof value.description === "string" ? value.description.trim() : "";
      const command = typeof value.command === "string" ? value.command.trim() : "";
      if (description || command) {
        return {
          summary:
            description ||
            (command.replace(/\s+/g, " ").length > 96
              ? `${command.replace(/\s+/g, " ").slice(0, 96)}…`
              : command.replace(/\s+/g, " ")),
          full: command || unfenced,
        };
      }
    } catch {
      // fall through
    }
  }
  const oneLine = full.replace(/\s+/g, " ");
  return {
    summary: oneLine.length > 96 ? `${oneLine.slice(0, 96)}…` : oneLine,
    full,
  };
}

export function looksLikeToolPayloadJson(text: string): boolean {
  const trimmed = text.trim();
  const unfenced = trimmed
    .replace(/^```(?:json)?\s*/i, "")
    .replace(/\s*```$/i, "")
    .trim();
  if (!unfenced.startsWith("{") || !unfenced.endsWith("}")) {
    return false;
  }
  try {
    const value = JSON.parse(unfenced) as Record<string, unknown>;
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      return false;
    }
    const hasToolShape =
      typeof value.command === "string" ||
      typeof value.description === "string" ||
      typeof value.tool === "string" ||
      typeof value.name === "string";
    return hasToolShape && !("role" in value) && !("content" in value);
  } catch {
    return false;
  }
}

export function activityKind(phase: string): "tool" | "think" | "write" | "info" | "error" {
  if (phase === "tool" || phase === "command" || phase === "permission") return "tool";
  if (phase === "thinking" || phase === "reasoning") return "think";
  if (phase === "writing" || phase === "streaming") return "write";
  if (phase === "error") return "error";
  return "info";
}

/** A browser or computer tool could not be connected for this turn. */
export const HAND_MISSING_PHASE = "hand_missing";

/** `<hand>:<reason>` then the original error. One sentence for the person, the rest for the log. */
export function handMissingNotice(message: string): { sentence: string; detail: string } {
  const breakAt = message.indexOf("\n");
  const head = (breakAt === -1 ? message : message.slice(0, breakAt)).trim();
  const detail = breakAt === -1 ? "" : message.slice(breakAt + 1).trim();
  const [hand, reason] = head.split(":");
  const browser = hand === "browser";
  let key: MessageKey;
  if (browser && reason === "chrome") key = "chat.handBrowserNoChrome";
  else if (reason === "cli") key = browser ? "chat.handBrowserOld" : "chat.handComputerOld";
  else key = browser ? "chat.handBrowserMissing" : "chat.handComputerMissing";
  return { sentence: t(key), detail };
}

/** Lifecycle chatter that belongs in the header live pill, not the transcript. */
export function isQuietPhase(phase: string): boolean {
  return phase === "writing" || phase === "streaming" || phase === "info" || phase === "done";
}

/** Short name on the first line; command or path on the following lines. */
export function splitToolActivity(text: string): { summary: string; detail: string } {
  const stripped = text.replace(/^(?:调用工具|call(?:ing)? tool)\s*/i, "").trim();
  const breakAt = stripped.indexOf("\n");
  // The start notice is "Bash…". The finished line is "Bash…\n<command>".
  // The dots belong to the notice, so both lines name the same tool.
  const head = (breakAt === -1 ? stripped : stripped.slice(0, breakAt)).trim().replace(/[….]+$/g, "");
  const detail = breakAt === -1 ? "" : stripped.slice(breakAt + 1).trim();
  const aliases = head.split("__").filter(Boolean);
  const summary = (aliases.length > 1 ? aliases[aliases.length - 1] : head) || text.trim();
  return { summary, detail };
}

export type ToolActivityKind = "read" | "write" | "edit" | "terminal" | "search" | "other";

export type ToolActivityInfo = {
  kind: ToolActivityKind;
  label: string;
  target: string;
  path: string;
  lineStart: number;
  lineEnd: number;
  additions: number;
  deletions: number;
  addedLines: string[];
  deletedLines: string[];
};

function toolTarget(text: string): {
  target: string;
  path: string;
  lineStart: number;
  lineEnd: number;
} {
  const lines = text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && line !== "@@diff" && !/^[+−-]\d+(?:\s+[+−-]\d+)?$/.test(line));
  const pathLine = lines.find((line) => {
    if (/^https?:\/\//i.test(line)) return false;
    return /(?:^|[/\\])[^/\\]+\.[a-z0-9]{1,12}(?:(?::\d+(?::\d+)?)|\s+L?\d+(?:[-–]\d+)?)?$/i.test(
      line,
    );
  });
  if (!pathLine) {
    return { target: lines[0] ?? "", path: "", lineStart: 0, lineEnd: 0 };
  }
  const location = pathLine.match(/(?:\s+L?(\d+)(?:[-–](\d+))?|:(\d+)(?::(\d+))?)$/i);
  const lineStart = Number(location?.[1] ?? location?.[3] ?? 0);
  const lineEnd = Number(location?.[2] ?? location?.[4] ?? lineStart);
  const clean = location ? pathLine.slice(0, location.index).trim() : pathLine;
  const name = clean.split(/[/\\]/).filter(Boolean).pop() ?? clean;
  return {
    target: lineStart
      ? `${name} · ${t("chat.fileLines", {
          start: String(lineStart),
          end: String(lineEnd || lineStart),
        })}`
      : name,
    path: clean,
    lineStart,
    lineEnd,
  };
}

function changeCounts(text: string): { additions: number; deletions: number } {
  const explicit = text.match(/(?:^|\s)\+(\d+)\s+[−-](\d+)(?:\s|$)/);
  if (explicit) {
    return { additions: Number(explicit[1]), deletions: Number(explicit[2]) };
  }
  let additions = 0;
  let deletions = 0;
  for (const line of text.split("\n")) {
    if (line.startsWith("+") && !line.startsWith("+++")) additions += 1;
    if (line.startsWith("-") && !line.startsWith("---")) deletions += 1;
  }
  return { additions, deletions };
}

function changedLines(text: string): { addedLines: string[]; deletedLines: string[] } {
  const marker = text.indexOf("\n@@diff\n");
  if (marker < 0) return { addedLines: [], deletedLines: [] };
  const addedLines: string[] = [];
  const deletedLines: string[] = [];
  for (const line of text.slice(marker + 8).split("\n")) {
    if (line.startsWith("+")) addedLines.push(line.slice(1));
    else if (line.startsWith("-")) deletedLines.push(line.slice(1));
  }
  return { addedLines, deletedLines };
}

const SHELL_PATH = String.raw`("[^"]+"|'[^']+'|[^\s;|&<>()]+)`;

function shellPath(raw: string): string {
  const path = raw.replace(/^["']|["']$/g, "");
  if (!path || path.startsWith("&") || path.startsWith("$") || path === "/dev/null") return "";
  const looksLikeFile = /[/\\]/.test(path) || /\.[a-z][a-z0-9]{0,9}$/i.test(path);
  return looksLikeFile ? path : "";
}

/** `cd /tmp && cat > lat.sh` names /tmp/lat.sh. A bare name is not in the project. */
function placeInShellDir(line: string, path: string): string {
  if (!path || path.startsWith("/") || path.startsWith("~/")) return path;
  const at = line.lastIndexOf(path);
  const before = at < 0 ? line.length : at;
  let dir = "";
  const cd = /(?:^|&&|\|\||;)\s*cd\s+(?:"([^"]+)"|'([^']+)'|([^\s;|&]+))/g;
  let match: RegExpExecArray | null;
  while ((match = cd.exec(line))) {
    if (match.index >= before) break;
    const target = match[1] || match[2] || match[3] || "";
    if (!target || target.startsWith("$")) continue;
    if (target.startsWith("/") || target.startsWith("~/")) {
      dir = target;
      continue;
    }
    if (!(dir.startsWith("/") || dir.startsWith("~/"))) continue;
    const base = dir.replace(/\/+$/, "");
    const rest = target.replace(/^\.\//, "");
    if (rest === "..") {
      const cut = base.lastIndexOf("/");
      dir = cut <= 0 ? "/" : base.slice(0, cut);
    } else if (rest !== ".") {
      dir = `${base}/${rest}`;
    }
  }
  if (!(dir.startsWith("/") || dir.startsWith("~/"))) return path;
  return `${dir.replace(/\/+$/, "")}/${path.replace(/^\.\//, "")}`;
}

function heredocLines(command: string): string[] {
  const start = command.match(/<<-?\s*['"]?(\w+)['"]?[^\n]*\n/);
  if (!start || start.index == null) return [];
  const body = command.slice(start.index + start[0].length).split("\n");
  const end = body.findIndex((line) => line.trim() === start[1]);
  return end < 0 ? body : body.slice(0, end);
}

/** A terminal command that writes, appends to, or reads a file names that file. */
function shellFileAction(
  command: string,
): { kind: "write" | "edit" | "read"; path: string; added: string[] } | null {
  const first = command.split("\n")[0] ?? "";
  const added = heredocLines(command);
  const echoed = first.match(/\b(?:echo|printf)\s+(?:-e\s+)?("[^"]*"|'[^']*')/)?.[1];
  const written = echoed ? [echoed.slice(1, -1)] : added;
  const tries: Array<["write" | "edit" | "read", RegExp]> = [
    ["edit", new RegExp(String.raw`>>\s*${SHELL_PATH}`)],
    ["edit", new RegExp(String.raw`\btee\s+-a\s+${SHELL_PATH}`)],
    ["edit", new RegExp(String.raw`\b(?:sed|perl)\s+-[a-z]*i\S*(?:\s+(?:''|""))?\s+(?:'[^']*'|"[^"]*"|\S+)\s+${SHELL_PATH}`)],
    ["write", new RegExp(String.raw`(?:^|[^>&\d])>\s*${SHELL_PATH}`)],
    ["write", new RegExp(String.raw`\btee\s+${SHELL_PATH}`)],
    ["write", new RegExp(String.raw`\btouch\s+${SHELL_PATH}`)],
    ["read", new RegExp(String.raw`\b(?:cat|head|tail|less|more|wc|nl|stat|md5|md5sum)\s+(?:-\S+\s+)*${SHELL_PATH}`)],
  ];
  for (const [kind, pattern] of tries) {
    const path = placeInShellDir(first, shellPath(first.match(pattern)?.[1] ?? ""));
    if (path) return { kind, path, added: kind === "read" ? [] : written };
  }
  return null;
}

/** Turn runtime-specific tool names into the stable actions shown in the timeline. */
export function toolActivityInfo(text: string): ToolActivityInfo {
  const { summary, detail } = splitToolActivity(text);
  const lower = `${summary}\n${detail}`.toLowerCase();
  const name = summary.toLowerCase().replace(/[^a-z0-9]/g, "");
  let kind: ToolActivityKind = "other";
  if (
    /^(websearch|webfetch|glob|grep|rg|search|searchfiles)$/.test(name) ||
    /(?:^|\s)(?:curl|wget|rg|grep)\s/.test(lower)
  ) {
    kind = "search";
  } else if (/^(read|readfile|view|imageview|openfile)$/.test(name)) {
    kind = "read";
  } else if (/^(write|writefile|createfile|addfile)$/.test(name)) {
    kind = "write";
  } else if (
    /^(edit|multiedit|notebookedit|applypatch|applydiff|filechange|workspacewrite|patch|strreplace|searchreplace)$/.test(
      name,
    )
  ) {
    kind = "edit";
  } else if (
    /^(bash|shell|terminal|command|commandexecution|exec|runcommand)$/.test(name) ||
    /(?:^|\s)(?:bash|zsh|sh|npm|pnpm|yarn|cargo|git|python|node)\s/.test(lower)
  ) {
    kind = "terminal";
  }
  if (kind === "terminal" || kind === "other") {
    const action = shellFileAction(detail || summary);
    if (action) {
      return presentTool(
        {
          kind: action.kind,
          label: toolKindLabel(action.kind),
          ...toolTarget(action.path),
          additions: action.added.length,
          deletions: 0,
          addedLines: action.added,
          deletedLines: [],
        },
        summary,
      );
    }
  }
  const counts = changeCounts(detail);
  const target = toolTarget(detail || summary);
  return presentTool(
    {
      kind,
      label: toolKindLabel(kind),
      ...target,
      ...counts,
      ...changedLines(detail),
    },
    summary,
  );
}

export function toolKindLabel(kind: ToolActivityKind): string {
  const keys: Record<ToolActivityKind, MessageKey> = {
    read: "chat.toolRead",
    write: "chat.toolWrite",
    edit: "chat.toolEdit",
    terminal: "chat.toolTerminal",
    search: "chat.toolSearch",
    other: "chat.toolOther",
  };
  return t(keys[kind]);
}

/** Built-in computer and browser tools, in words a beginner already has. */
const TOOL_ALIASES: Record<string, MessageKey> = {
  screen_apps: "chat.alias.screenApps",
  screen_open: "chat.alias.screenOpen",
  screen_snapshot: "chat.alias.screenSnapshot",
  screen_click: "chat.alias.screenClick",
  screen_type: "chat.alias.screenType",
  screen_press: "chat.alias.screenPress",
  screen_scroll: "chat.alias.screenScroll",
  screen_windows: "chat.alias.screenWindows",
  screen_state: "chat.alias.screenState",
  screen_set: "chat.alias.screenSet",
  screen_menu: "chat.alias.screenMenu",
  browser_navigate: "chat.alias.browserOpen",
  browser_snapshot: "chat.alias.browserLook",
  browser_screenshot: "chat.alias.browserLook",
  browser_click: "chat.alias.browserClick",
  browser_fill: "chat.alias.browserFill",
  browser_type: "chat.alias.browserType",
  browser_find: "chat.alias.browserFind",
  browser_scroll: "chat.alias.browserScroll",
  browser_wait: "chat.alias.browserWait",
  browser_get_text: "chat.alias.browserText",
  browser_get_url: "chat.alias.browserUrl",
  browser_get_html: "chat.alias.browserPage",
  browser_evaluate: "chat.alias.browserPage",
  browser_get_links: "chat.alias.browserLinks",
  browser_new_tab: "chat.alias.browserTab",
  browser_switch_tab: "chat.alias.browserTab",
  browser_close_tab: "chat.alias.browserTab",
  browser_list_tabs: "chat.alias.browserTabs",
  browser_state_save: "chat.alias.browserRemember",
  browser_state_load: "chat.alias.browserRemember",
};

const KIND_WORDS = new Set([
  "read",
  "readfile",
  "view",
  "write",
  "writefile",
  "edit",
  "bash",
  "shell",
  "grep",
  "glob",
  "search",
]);

function bareToolName(raw: string): string {
  const parts = raw
    .split("__")
    .flatMap((part) => part.split("/"))
    .map((part) => part.trim())
    .filter(Boolean);
  return (parts[parts.length - 1] ?? "").toLowerCase().replace(/[^a-z0-9_]/g, "");
}

/** `mcp__computer__screen_snapshot` reads as `看屏幕`. Empty when there is no alias. */
export function toolCallAlias(...parts: string[]): string {
  for (const part of parts) {
    const key = TOOL_ALIASES[bareToolName(part)];
    if (key) return t(key);
  }
  return "";
}

function hideRawToolName(shown: string): boolean {
  const text = shown.trim();
  if (!text || /[\\/]/.test(text)) return false;
  const bare = bareToolName(text);
  return Boolean(TOOL_ALIASES[bare]) || text.includes("__") || KIND_WORDS.has(bare);
}

/** English names of apps that ship with the system, in words people already use. */
const FAMILIAR_APPS: Record<string, MessageKey> = {
  "system settings": "chat.app.systemSettings",
  "system preferences": "chat.app.systemSettings",
  "com.apple.systempreferences": "chat.app.systemSettings",
  finder: "chat.app.finder",
  "com.apple.finder": "chat.app.finder",
  notes: "chat.app.notes",
  "com.apple.notes": "chat.app.notes",
  calendar: "chat.app.calendar",
  "com.apple.ical": "chat.app.calendar",
  mail: "chat.app.mail",
  "com.apple.mail": "chat.app.mail",
  messages: "chat.app.messages",
  "com.apple.mobilesms": "chat.app.messages",
  photos: "chat.app.photos",
  "com.apple.photos": "chat.app.photos",
  music: "chat.app.music",
  "com.apple.music": "chat.app.music",
  maps: "chat.app.maps",
  "com.apple.maps": "chat.app.maps",
  reminders: "chat.app.reminders",
  "com.apple.reminders": "chat.app.reminders",
  contacts: "chat.app.contacts",
  "com.apple.addressbook": "chat.app.contacts",
  preview: "chat.app.preview",
  "com.apple.preview": "chat.app.preview",
  textedit: "chat.app.textEdit",
  "com.apple.textedit": "chat.app.textEdit",
  terminal: "chat.app.terminal",
  "com.apple.terminal": "chat.app.terminal",
  "activity monitor": "chat.app.activityMonitor",
  "com.apple.activitymonitor": "chat.app.activityMonitor",
  "disk utility": "chat.app.diskUtility",
  "com.apple.diskutility": "chat.app.diskUtility",
  shortcuts: "chat.app.shortcuts",
  "com.apple.shortcuts": "chat.app.shortcuts",
  calculator: "chat.app.calculator",
  "com.apple.calculator": "chat.app.calculator",
  dictionary: "chat.app.dictionary",
  "com.apple.dictionary": "chat.app.dictionary",
  podcasts: "chat.app.podcasts",
  "com.apple.podcasts": "chat.app.podcasts",
  news: "chat.app.news",
  "com.apple.news": "chat.app.news",
  books: "chat.app.books",
  "com.apple.ibooks": "chat.app.books",
  "com.apple.ibooksx": "chat.app.books",
  weather: "chat.app.weather",
  "com.apple.weather": "chat.app.weather",
  clock: "chat.app.clock",
  "com.apple.clock": "chat.app.clock",
  "voice memos": "chat.app.voiceMemos",
  "com.apple.voicememos": "chat.app.voiceMemos",
  "font book": "chat.app.fontBook",
  "com.apple.fontbook": "chat.app.fontBook",
  "keychain access": "chat.app.keychain",
  "com.apple.keychainaccess": "chat.app.keychain",
  "system information": "chat.app.systemInfo",
  "com.apple.systemprofiler": "chat.app.systemInfo",
  console: "chat.app.console",
  "com.apple.console": "chat.app.console",
  passwords: "chat.app.passwords",
  "com.apple.passwords": "chat.app.passwords",
  freeform: "chat.app.freeform",
  "com.apple.freeform": "chat.app.freeform",
  stickies: "chat.app.stickies",
  "com.apple.stickies": "chat.app.stickies",
  launchpad: "chat.app.launchpad",
  "mission control": "chat.app.missionControl",
  settings: "chat.app.settings",
  "file explorer": "chat.app.explorer",
  explorer: "chat.app.explorer",
  notepad: "chat.app.notepad",
  "task manager": "chat.app.taskManager",
  "control panel": "chat.app.controlPanel",
  paint: "chat.app.paint",
  "snipping tool": "chat.app.snip",
};

function familiarApp(raw: string): string {
  const key = raw.trim().toLowerCase();
  const id = FAMILIAR_APPS[key];
  return id ? t(id) : raw.trim();
}

const ELEMENT_LINE =
  /^@e\d+\s+(?:button|text field|text|link|image|checkbox|tab)\s+(.+)$/i;

/**
 * A screen row is `@e3 button 全部清除 315 582`. The row shows the words.
 * A dumped list after an app name stays off the row.
 */
function surfaceName(text: string): string {
  const raw = text.trim();
  if (!raw) return "";
  if (!/@e\d+\b/.test(raw) && !raw.includes("\\n")) return raw;
  const flat = raw.replace(/\\n/g, "\n").replace(/\\t/g, " ").trim();
  const only = flat.replace(/\s*\n\s*/g, " ").trim();
  const element = !flat.includes("\n") ? only.match(ELEMENT_LINE) : null;
  if (element) {
    return element[1].replace(/\s+-?\d+(?:\.\d+)?\s+-?\d+(?:\.\d+)?\s*$/, "").trim();
  }
  for (const line of flat.split("\n")) {
    const row = line.trim();
    if (!row || /^@e\d+\b/.test(row)) continue;
    if (/^(rows are|screenshot file|image pixels|the picture|click x,y)\b/i.test(row)) continue;
    const app = row.match(/^(?:front app|window)\s*:\s*(.+)$/i)?.[1]?.trim() ?? row;
    const cut = app.split(/@e\d+\b/)[0]?.trim() ?? "";
    if (cut) return cut;
  }
  return "";
}

function quoteName(text: string): string {
  const clean = surfaceName(text).replace(/^「|」$/g, "");
  return clean ? `「${oneLine(clean, 24)}」` : "";
}

function quoteApp(text: string): string {
  return quoteName(familiarApp(text));
}

function seenApp(output: string): string {
  const windowName = output.match(/window:\s*([^\n"]+)/)?.[1]?.trim() ?? "";
  const app = output.match(/front app:\s*([^\n"]+)/)?.[1]?.trim() ?? "";
  const opened = output.match(/"opened"\s*:\s*"([^"]+)"/)?.[1]?.trim() ?? "";
  const bundle = output.match(/"bundleId"\s*:\s*"([^"]+)"/)?.[1]?.trim() ?? "";
  return [opened, windowName, app, bundle].find((name) => name.length > 0) ?? "";
}

/** A computer-tool result that only repeats what the row already says. */
export function isMachineToolDump(output: string): boolean {
  const text = output.trim();
  if (!text.startsWith("{")) return false;
  try {
    const value = JSON.parse(text) as Record<string, unknown>;
    const keys = Object.keys(value);
    if (keys.length === 0) return false;
    const allowed = new Set([
      "opened",
      "bundleId",
      "path",
      "clicked",
      "pressed",
      "typed",
      "scrolled",
      "x",
      "y",
      "amount",
      "set",
      "menu",
      "apps",
    ]);
    return keys.every((key) => allowed.has(key));
  } catch {
    return false;
  }
}

function plainOutcome(output: string): string {
  let body = output.trim();
  if (!body) return "";
  if (body.startsWith("{")) {
    try {
      const value = JSON.parse(body) as { text?: unknown; error?: unknown };
      body = typeof value.text === "string" ? value.text : typeof value.error === "string" ? value.error : "";
    } catch {
      body = "";
    }
  }
  const line = body
    .split("\n")
    .map((row) => row.trim())
    .find((row) => {
      if (!/[\u4e00-\u9fff]/.test(row) || row.includes("\t") || /@e\d+\b/.test(row)) return false;
      return !/^(front app|window|rows are|screenshot|the picture)\b/i.test(row);
    });
  if (!line) return "";
  const sentence = line.split(/[。！？]/)[0]?.trim() ?? "";
  return sentence.length > 32 ? `${sentence.slice(0, 32)}…` : sentence;
}

function keyPhrase(raw: string): string {
  const key = raw.trim().toLowerCase().replace(/\s+/g, "");
  const known: Record<string, MessageKey> = {
    return: "chat.key.return",
    enter: "chat.key.return",
    escape: "chat.key.escape",
    esc: "chat.key.escape",
    tab: "chat.key.tab",
    space: "chat.key.space",
    up: "chat.key.up",
    down: "chat.key.down",
    left: "chat.key.left",
    right: "chat.key.right",
    backspace: "chat.key.backspace",
    bksp: "chat.key.backspace",
    bs: "chat.key.backspace",
    delete: "chat.key.delete",
    del: "chat.key.delete",
    "command+a": "chat.key.selectAll",
    "cmd+a": "chat.key.selectAll",
    "ctrl+a": "chat.key.selectAll",
    "command+c": "chat.key.copy",
    "cmd+c": "chat.key.copy",
    "ctrl+c": "chat.key.copy",
    "command+v": "chat.key.paste",
    "cmd+v": "chat.key.paste",
    "ctrl+v": "chat.key.paste",
    "command+z": "chat.key.undo",
    "cmd+z": "chat.key.undo",
    "ctrl+z": "chat.key.undo",
  };
  const id = known[key];
  return id ? t(id) : raw.trim();
}

function scrollPhrase(raw: string): string {
  const key = raw.trim().toLowerCase();
  if (key === "up" || raw === "向上") return t("chat.key.up");
  if (key === "down" || raw === "向下") return t("chat.key.down");
  return raw.trim();
}

/** One sentence a beginner can read. Empty when this is not a screen or page tool. */
function actionLine(leaf: string, subject: string, output: string): string {
  const named = surfaceName(subject);
  const seen = named || surfaceName(seenApp(output));
  const quoted = quoteName(named);
  const seenQuoted = quoteApp(seen);
  switch (leaf) {
    case "screen_snapshot":
      return seenQuoted ? t("chat.line.lookApp", { name: seenQuoted }) : t("chat.line.look");
    case "screen_open": {
      const opened = quoteApp(named || seenApp(output));
      return opened ? t("chat.line.open", { name: opened }) : t("chat.line.openAny");
    }
    case "screen_click":
    case "browser_click":
      return quoted ? t("chat.line.click", { name: quoteApp(named) }) : t("chat.line.clickSpot");
    case "screen_type":
    case "browser_type":
    case "browser_fill":
      return quoted ? t("chat.line.type", { name: quoted }) : t("chat.line.typeAny");
    case "screen_press":
      return t("chat.line.press", { name: named ? keyPhrase(named) : t("chat.key.return") });
    case "screen_scroll":
    case "browser_scroll":
      return named ? t("chat.line.scroll", { name: scrollPhrase(named) }) : t("chat.line.scrollAny");
    case "screen_windows":
      return seenQuoted ? t("chat.line.windows", { name: seenQuoted }) : t("chat.line.windowsAny");
    case "screen_state":
      return seenQuoted ? t("chat.line.state", { name: seenQuoted }) : t("chat.line.stateAny");
    case "screen_set":
      return quoted ? t("chat.line.set", { name: quoted }) : t("chat.line.setAny");
    case "screen_menu":
      return quoted ? t("chat.line.menu", { name: quoted }) : t("chat.line.menuAny");
    case "screen_apps":
      return t("chat.line.apps");
    case "browser_navigate":
      return quoted ? t("chat.line.page", { name: quoted }) : t("chat.line.pageAny");
    case "browser_snapshot":
    case "browser_screenshot":
      return t("chat.line.pageLook");
    case "browser_find":
      return quoted ? t("chat.line.find", { name: quoted }) : t("chat.line.find", { name: "" }).trim();
    case "browser_wait":
      return t("chat.line.wait");
    default:
      return "";
  }
}

/** The row says what happened. The raw tool id stays off the surface. */
function presentTool(info: ToolActivityInfo, rawName: string, output = ""): ToolActivityInfo {
  const subject = info.target && hideRawToolName(info.target) ? "" : info.target;
  const line = actionLine(bareToolName(rawName) || bareToolName(info.target), subject, output);
  if (line && !info.path) {
    const outcome = plainOutcome(output);
    const label = outcome && !line.includes(outcome) ? `${line} · ${outcome}` : line;
    return { ...info, label, target: "" };
  }
  const alias = toolCallAlias(rawName, info.target);
  if (alias && !info.path) return { ...info, label: alias, target: subject };
  if (subject !== info.target && !info.path) return { ...info, target: subject };
  return info;
}

/** How many times each row should show. `0` means it repeats the row before it. */
export function repeatRuns(faces: string[]): number[] {
  const counts = faces.map(() => 0);
  let index = 0;
  while (index < faces.length) {
    let next = index + 1;
    while (next < faces.length && faces[next] === faces[index]) next += 1;
    counts[index] = next - index;
    index = next;
  }
  return counts;
}

const STEP_KINDS: ToolActivityKind[] = ["read", "write", "edit", "terminal", "search", "other"];

const subagentNames = new Map<string, string>();

/** The name the user wrote, keyed by the id each assistant actually calls. */
export function rememberSubagentNames(records: { id: string; name: string }[]): void {
  subagentNames.clear();
  for (const record of records) {
    const name = record.name.trim();
    if (!record.id || !name) continue;
    subagentNames.set(`role-${record.id}`, name);
    subagentNames.set(`ad-role-${record.id}`, name);
  }
}

function subagentFace(title: string): string {
  const who = title.replace(/\s+/g, " ").trim();
  if (!who) return t("chat.subagentSomeone");
  const known = subagentNames.get(who) ?? who;
  return known.length > 40 ? `${known.slice(0, 40)}…` : known;
}

function oneLine(text: string, max = 96): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > max ? `${line.slice(0, max)}…` : line;
}

/** The row for a structured step: what it did, and to which file. */
export function stepActivityInfo(step: ToolStep): ToolActivityInfo {
  if (step.kind === "subagent") {
    const name = subagentFace(step.title || "");
    return {
      kind: "other",
      label: t("chat.subagentWatch", { name }),
      target: "",
      path: "",
      lineStart: 0,
      lineEnd: 0,
      additions: 0,
      deletions: 0,
      addedLines: [],
      deletedLines: [],
    };
  }
  const kind = STEP_KINDS.includes(step.kind as ToolActivityKind)
    ? (step.kind as ToolActivityKind)
    : "other";
  const path = placeInShellDir(step.command ?? "", step.path?.trim() ?? "");
  const name = path.split(/[/\\]/).filter(Boolean).pop() ?? path;
  const lineStart = step.line_start ?? 0;
  const lineEnd = step.line_end ?? lineStart;
  const fileKind = kind === "read" || kind === "write" || kind === "edit";
  let target: string;
  if (fileKind && path) {
    target = lineStart
      ? `${name} · ${t("chat.fileLines", { start: String(lineStart), end: String(lineEnd) })}`
      : name;
  } else if (kind === "search") {
    target = oneLine(step.title || step.query || step.command || "");
  } else {
    target = oneLine(step.title || step.command || step.query || step.name || "");
  }
  return presentTool(
    {
      kind,
      label: toolKindLabel(kind),
      target,
      path: fileKind ? path : "",
      lineStart,
      lineEnd,
      additions: step.additions ?? 0,
      deletions: step.deletions ?? 0,
      addedLines: step.added_lines ?? [],
      deletedLines: step.deleted_lines ?? [],
    },
    step.name || step.title || "",
    step.output ?? "",
  );
}

/** A result update only carries what it adds. Keep the rest from the start. */
export function mergeToolStep(base: ToolStep | undefined, update: ToolStep): ToolStep {
  if (!base) return update;
  const merged: Record<string, unknown> = { ...base };
  for (const [key, value] of Object.entries(update)) {
    if (value === undefined || value === null || value === "") continue;
    if (Array.isArray(value) && value.length === 0) continue;
    merged[key] = value;
  }
  return merged as unknown as ToolStep;
}

/** Text kept for older readers (context, records) when a step has no status line. */
export function toolStepText(step: ToolStep): string {
  const detail = step.command || step.path || step.query || "";
  const name = step.name || step.kind || "tool";
  return detail ? `调用工具 ${name}…\n${detail}` : `调用工具 ${name}…`;
}

export function readRowStep(row: HTMLElement): ToolStep | undefined {
  const raw = row.dataset.step;
  if (!raw) return undefined;
  try {
    return JSON.parse(raw) as ToolStep;
  } catch {
    return undefined;
  }
}

/** Structured rows read their step; older rows still parse the status line. */
export function rowActivityInfo(row: HTMLElement): ToolActivityInfo {
  const step = readRowStep(row);
  if (step) return stepActivityInfo(step);
  const name = row.dataset.summary ?? "";
  const detail = row.dataset.detail ?? "";
  return toolActivityInfo(detail ? `${name}\n${detail}` : name);
}

export function summarizeToolActivities(infos: ToolActivityInfo[]): string {
  const counts = new Map<ToolActivityKind, number>();
  const files = new Map<ToolActivityKind, Set<string>>();
  const named = new Map<string, number>();
  const genericOther = toolKindLabel("other");
  for (const info of infos) {
    if (info.kind === "read" || info.kind === "write" || info.kind === "edit") {
      const seen = files.get(info.kind) ?? new Set<string>();
      seen.add(info.target || `${info.kind}-${seen.size}`);
      files.set(info.kind, seen);
      counts.set(info.kind, seen.size);
    } else if (info.kind === "other" && info.label !== genericOther) {
      named.set(info.label, (named.get(info.label) ?? 0) + 1);
    } else {
      counts.set(info.kind, (counts.get(info.kind) ?? 0) + 1);
    }
  }
  const parts: string[] = [];
  const add = (kind: ToolActivityKind, key: MessageKey) => {
    const count = counts.get(kind) ?? 0;
    if (count > 0) parts.push(t(key, { count: String(count) }));
  };
  add("read", "chat.workReadFiles");
  add("write", "chat.workWroteFiles");
  add("edit", "chat.workEditedFiles");
  add("search", "chat.workSearches");
  add("terminal", "chat.workCommands");
  for (const [name, count] of named) {
    parts.push(count > 1 ? t("chat.workAliasCount", { name, count: String(count) }) : name);
  }
  add("other", "chat.workOtherActions");
  const additions = infos.reduce((sum, info) => sum + info.additions, 0);
  const deletions = infos.reduce((sum, info) => sum + info.deletions, 0);
  if (additions || deletions) parts.push(`+${additions} −${deletions}`);
  return parts.join(" · ");
}

export function cleanToolLabel(text: string): string {
  return splitToolActivity(text).summary;
}

export function toolSignature(text: string): string {
  const { summary, detail } = splitToolActivity(text);
  return `${summary}\n${detail}`.toLocaleLowerCase();
}

/** A short closing question that is explicitly asking to continue. */
export function looksLikeChoiceQuestion(text: string): boolean {
  const trimmed = text.trim();
  if (!trimmed) return false;
  const plain = trimmed.replace(/\s+/g, " ");
  const match = plain.match(/[^。！？!?]{0,40}[？?]\s*$/);
  if (!match) return false;
  const ask = match[0].trim();
  if (
    /有什么(可以|需要|能)?(帮|帮忙|做)/.test(ask) ||
    /需要我(帮忙|帮你)/.test(ask) ||
    /随时(找我|叫我|告诉我)/.test(ask)
  ) {
    return false;
  }
  const lower = ask.toLowerCase();
  return (
    /(要不要继续|还要继续|继续吗|要我继续|需要我继续|确认继续|接着做吗)/.test(ask) ||
    /(shall i continue|should i continue|want me to continue|proceed\?)/i.test(lower)
  );
}

/** One-line surfaces (the island list) show the words, not the marks. */
export function stripMarkdown(text: string): string {
  return text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)]\([^)]*\)/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/(^|[\s])\*([^*\n]+)\*(?=$|[\s])/g, "$1$2")
    .replace(/`/g, "")
    .replace(/\*\*/g, "")
    .replace(/^\s*[-*+]\s+/gm, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** Folded preview: drop the marks, keep each paragraph on its own line. */
export function foldPreview(text: string): string {
  return text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)]\([^)]*\)/g, "$1")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/__([^_]+)__/g, "$1")
    .replace(/(^|[\s])\*([^*\n]+)\*(?=$|[\s])/g, "$1$2")
    .replace(/`/g, "")
    .replace(/\*\*/g, "")
    .replace(/^\s*[-*+]\s+/gm, "")
    .replace(/[ \t]+/g, " ")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .join("\n")
    .trim();
}

export function preferPlainSummary(summary: string): string {
  const trimmed = summary.trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("{")) {
    try {
      const parsed = JSON.parse(trimmed) as { result?: unknown };
      if (typeof parsed.result === "string" && parsed.result.trim()) {
        return parsed.result.trim();
      }
    } catch {
      for (const line of trimmed.split("\n").reverse()) {
        try {
          const parsed = JSON.parse(line) as { type?: string; result?: unknown; is_error?: boolean };
          if (
            parsed.type === "result" &&
            !parsed.is_error &&
            typeof parsed.result === "string" &&
            parsed.result.trim()
          ) {
            return parsed.result.trim();
          }
        } catch {
          /* continue */
        }
      }
    }
    return "";
  }
  if (/^[\w.-]+ ask ended \([^)]*\) with no captured output$/i.test(trimmed)) return "";
  if (/^claude-code (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^codex (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^hermes (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^openclaw (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  if (/^deepseek-harness (completed|failed|cancelled|timed out)$/i.test(trimmed)) return "";
  return trimmed;
}

export function shortCwdLabel(cwd: string): string {
  const trimmed = cwd.trim();
  if (!trimmed || trimmed === "—") return "—";
  const parts = trimmed.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || trimmed;
}
