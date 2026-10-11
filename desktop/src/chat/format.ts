import { t, type MessageKey } from "../i18n";
import type { ToolStep } from "./types";

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

/** Lifecycle chatter that belongs in the header live pill, not the transcript. */
export function isQuietPhase(phase: string): boolean {
  return phase === "writing" || phase === "streaming" || phase === "info" || phase === "done";
}

/** Short name on the first line; command or path on the following lines. */
export function splitToolActivity(text: string): { summary: string; detail: string } {
  const stripped = text
    .replace(/^(?:调用工具|call(?:ing)? tool)\s*/i, "")
    .replace(/[….\s]+$/g, "")
    .trim();
  const breakAt = stripped.indexOf("\n");
  const head = (breakAt === -1 ? stripped : stripped.slice(0, breakAt)).trim();
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
    const path = shellPath(first.match(pattern)?.[1] ?? "");
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
    /^(edit|multiedit|notebookedit|applypatch|filechange|workspacewrite)$/.test(name)
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
      return {
        kind: action.kind,
        label: toolKindLabel(action.kind),
        ...toolTarget(action.path),
        additions: action.added.length,
        deletions: 0,
        addedLines: action.added,
        deletedLines: [],
      };
    }
  }
  const counts = changeCounts(detail);
  const target = toolTarget(detail || summary);
  return {
    kind,
    label: toolKindLabel(kind),
    ...target,
    ...counts,
    ...changedLines(detail),
  };
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

const STEP_KINDS: ToolActivityKind[] = ["read", "write", "edit", "terminal", "search", "other"];

function oneLine(text: string, max = 96): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > max ? `${line.slice(0, max)}…` : line;
}

/** The row for a structured step: what it did, and to which file. */
export function stepActivityInfo(step: ToolStep): ToolActivityInfo {
  const kind = STEP_KINDS.includes(step.kind as ToolActivityKind)
    ? (step.kind as ToolActivityKind)
    : "other";
  const path = step.path?.trim() ?? "";
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
  return {
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
  };
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
  for (const info of infos) {
    if (info.kind === "read" || info.kind === "write" || info.kind === "edit") {
      const seen = files.get(info.kind) ?? new Set<string>();
      seen.add(info.target || `${info.kind}-${seen.size}`);
      files.set(info.kind, seen);
      counts.set(info.kind, seen.size);
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

export function isQuietStderr(line: string): boolean {
  const text = line.trim();
  const lower = text.toLowerCase();
  return (
    /^session_id:/i.test(text) ||
    /^resume this session/i.test(text) ||
    /resumed session/i.test(text) ||
    lower.includes("unrecognized_model") ||
    lower.includes("unrecognized model") ||
    lower.startsWith("[agent/embedded]") ||
    lower.includes("preserved orphaned user message") ||
    lower.includes("network connection was interrupted") ||
    lower.includes("transient same-model retry") ||
    lower.startsWith("[secrets]") ||
    lower.includes("secrets.resolve unavailable") ||
    lower.includes("resolved command secrets locally") ||
    (lower.includes("cua-driver") &&
      (lower.includes("is available") ||
        lower.includes("update with") ||
        lower.includes("release notes") ||
        lower.includes("mcp launched") ||
        lower.includes("tcc") ||
        lower.includes("auto-launching") ||
        lower.includes("proxying mcp"))) ||
    lower.includes("openclaw gateway run") ||
    lower.includes("openclaw gateway status") ||
    lower.startsWith("gateway target:") ||
    lower.startsWith("source: local loopback") ||
    lower.startsWith("bind: loopback") ||
    (lower.startsWith("config:") && lower.includes("openclaw.json"))
  );
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
