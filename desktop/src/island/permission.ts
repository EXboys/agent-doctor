import { t } from "../i18n";
import type { IslandPending } from "./track";

export type PermissionLineKind = "ctx" | "add" | "del";

export type PermissionLine = { kind: PermissionLineKind; text: string };

/** What an Allow request will do, in words a beginner already has. */
export type PermissionView = {
  verb: string;
  target: string;
  note: string;
  lines: PermissionLine[];
  more: number;
};

type ToolKind = "run" | "edit" | "write" | "read" | "web" | "browser" | "other";

const MAX_LINES = 6;
const MAX_LINE_CHARS = 160;

function inputOf(pending: IslandPending): Record<string, unknown> {
  const raw = pending.inputJson?.trim();
  if (!raw) return {};
  try {
    const value = JSON.parse(raw) as unknown;
    return value && typeof value === "object" && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

function text(value: unknown): string {
  if (typeof value === "string") return value.trim();
  if (Array.isArray(value) && value.every((part) => typeof part === "string")) {
    return value.join(" ").trim();
  }
  return "";
}

function firstText(input: Record<string, unknown>, keys: string[]): string {
  for (const key of keys) {
    const value = text(input[key]);
    if (value) return value;
  }
  return "";
}

function toolKind(tool: string): ToolKind {
  const name = tool.toLowerCase().replace(/[^a-z0-9_]/g, "");
  if (name.includes("browser")) return "browser";
  if (["bash", "shell", "exec", "command", "commandexecution", "localshell"].includes(name)) return "run";
  if (["edit", "multiedit", "notebookedit", "filechange", "applypatch", "apply_patch", "str_replace_editor"].includes(name)) {
    return "edit";
  }
  if (name === "write" || name === "writefile") return "write";
  if (["read", "glob", "grep", "ls", "readfile"].includes(name)) return "read";
  if (name === "webfetch" || name === "websearch") return "web";
  return "other";
}

/** `mcp__github__create_issue` reads as `github`. */
function friendlyTool(tool: string): string {
  const parts = tool.split("__").filter(Boolean);
  if (parts[0] === "mcp" && parts.length >= 2) return parts[1];
  return tool || "tool";
}

function clip(line: string): string {
  return line.length > MAX_LINE_CHARS ? `${line.slice(0, MAX_LINE_CHARS - 1)}…` : line;
}

function linesOf(body: string, kind: PermissionLineKind): PermissionLine[] {
  return body
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => ({ kind, text: clip(line.replace(/\t/g, "  ")) }));
}

function editLines(input: Record<string, unknown>): PermissionLine[] {
  const edits = Array.isArray(input.edits) ? input.edits : [input];
  const out: PermissionLine[] = [];
  for (const edit of edits) {
    if (!edit || typeof edit !== "object") continue;
    const record = edit as Record<string, unknown>;
    const before = text(record.old_string);
    const after = text(record.new_string);
    if (before) out.push(...linesOf(before, "del"));
    if (after) out.push(...linesOf(after, "add"));
  }
  return out;
}

export function permissionView(pending: IslandPending): PermissionView {
  const input = inputOf(pending);
  const tool = pending.tool ?? "";
  const kind = toolKind(tool);
  const description = text(input.description);
  let target = "";
  let lines: PermissionLine[] = [];

  if (kind === "run") {
    const command = firstText(input, ["command", "cmd"]);
    const all = command ? linesOf(command, "ctx") : [];
    target = all[0]?.text ?? "";
    lines = all.length > 1 || target.length > 64 ? all : [];
  } else if (kind === "edit") {
    target = firstText(input, ["file_path", "path", "notebook_path"]);
    lines = editLines(input);
  } else if (kind === "write") {
    target = firstText(input, ["file_path", "path"]);
    const content = text(input.content);
    lines = content ? linesOf(content, "add") : [];
  } else if (kind === "read") {
    target = firstText(input, ["file_path", "path", "pattern"]);
  } else if (kind === "web") {
    target = firstText(input, ["url", "query"]);
  } else if (kind === "browser") {
    target = firstText(input, ["url", "element", "text"]);
  }

  const verb =
    kind === "run"
      ? t("island.toolRun")
      : kind === "edit"
        ? t("island.toolEdit")
        : kind === "write"
          ? t("island.toolWrite")
          : kind === "read"
            ? t("island.toolRead")
            : kind === "web"
              ? t("island.toolWeb")
              : kind === "browser"
                ? t("island.toolBrowser")
                : t("island.toolOther", { tool: friendlyTool(tool) });

  const summary = pending.detail.trim();
  if (!target && summary && summary !== description) target = summary;
  const note = description && description !== target ? description : "";
  const shown = lines.slice(0, MAX_LINES);
  return { verb, target, note, lines: shown, more: Math.max(0, lines.length - shown.length) };
}
