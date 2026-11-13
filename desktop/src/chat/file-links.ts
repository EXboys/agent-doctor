import { findWorkspacePath, type WorkspaceMatch } from "../ipc";

let rootOf: () => string = () => "";
const settled = new Map<string, WorkspaceMatch | null>();
const pending = new Map<string, Promise<WorkspaceMatch | null>>();

/** How many path lookups may be in flight. Each one can walk a project tree. */
const MAX_IN_FLIGHT = 2;

type LinkJob = { root: string; query: string; code: HTMLElement };
const queue: LinkJob[] = [];
let inFlight = 0;
let pumpScheduled = false;

export function setFileLinkRoot(getRoot: () => string): void {
  rootOf = getRoot;
}

export function fileLinkRoot(): string {
  return rootOf().trim();
}

const FILE_EXTENSIONS = new Set([
  "ts", "tsx", "mts", "cts", "js", "mjs", "cjs", "jsx", "json", "jsonc", "jsonl",
  "md", "mdx", "txt", "log", "pdf", "doc", "docx", "rtf", "xls", "xlsx", "xlsm",
  "csv", "tsv", "ppt", "pptx", "png", "jpg", "jpeg", "gif", "webp", "svg", "ico",
  "bmp", "avif", "heic", "mp3", "wav", "m4a", "ogg", "flac", "mp4", "mov", "webm",
  "mkv", "zip", "tar", "gz", "tgz", "7z", "rar", "dmg", "pkg", "rs", "py", "go",
  "java", "kt", "kts", "swift", "c", "h", "cpp", "cc", "hpp", "cs", "rb", "php",
  "sql", "sh", "bash", "zsh", "fish", "ps1", "yaml", "yml", "toml", "ini", "cfg",
  "conf", "env", "lock", "html", "htm", "css", "scss", "less", "vue", "svelte",
  "xml", "plist", "proto", "graphql", "wasm", "db", "sqlite", "ipynb", "lua",
  "dart", "zig", "mdc",
]);

function fileExtension(name: string): string {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return "";
  return name.slice(dot + 1).toLowerCase();
}

/** `console.log` and `res.json` are code, not files. `package.json` is a file. */
const CODE_RECEIVERS = new Set([
  "console", "math", "window", "document", "json", "object", "array", "string",
  "number", "promise", "process", "module", "exports", "res", "req", "response",
  "request", "err", "error", "data", "result", "deps", "this", "self", "event",
]);

/**
 * `digest.mjs`, `logs/a.md`, `.daily-digest`, `~/x/y.json`.
 * Not `console.log`, `chatState.store`, `yes/no`, or `x1/56ms`.
 */
export function looksLikeFileMention(text: string): boolean {
  const value = text.trim();
  if (!value || value.length > 240 || /[\n<>|*?`()[\]{};=,"']/.test(value)) return false;
  if (/^https?:\/\//i.test(value)) return false;
  if (/^\w+\/\d/.test(value)) return false;
  const last = value.replace(/\/+$/, "").split("/").pop() ?? "";
  const ext = fileExtension(last);
  const receiver = last.slice(0, Math.max(0, last.length - ext.length - 1)).toLowerCase();
  const hasExtension = FILE_EXTENSIONS.has(ext) && !CODE_RECEIVERS.has(receiver);
  const dotFolder = /^\.[a-z0-9][\w.-]*$/i.test(last);
  if (!value.includes("/")) return hasExtension || dotFolder;
  if (hasExtension || dotFolder) return true;
  if (/^(?:\/|~\/|\.\.?\/)/.test(value) || value.endsWith("/")) return true;
  return value.split("/").length >= 3;
}

function cacheKey(root: string, query: string): string {
  return `${root}\u0001${query}`;
}

function lookup(root: string, query: string): Promise<WorkspaceMatch | null> {
  const key = cacheKey(root, query);
  if (settled.has(key)) return Promise.resolve(settled.get(key) ?? null);
  let found = pending.get(key);
  if (!found) {
    found = findWorkspacePath({ root, query })
      .catch(() => null)
      .then((match) => {
        settled.set(key, match);
        pending.delete(key);
        return match;
      });
    pending.set(key, found);
  }
  return found;
}

function applyMatch(code: HTMLElement, query: string, match: WorkspaceMatch): void {
  if (!code.isConnected || code.classList.contains("chat-file-link")) return;
  code.classList.add("chat-file-link");
  code.dataset.isDir = match.isDir ? "1" : "";
  code.setAttribute("role", "link");
  code.tabIndex = 0;
  const open = (event: Event) => {
    event.preventDefault();
    event.stopPropagation();
    window.dispatchEvent(
      new CustomEvent("chat-open-workspace-file", {
        detail: { path: query },
      }),
    );
  };
  code.addEventListener("click", open);
  code.addEventListener("keydown", (event) => {
    if (event.key === "Enter") open(event);
  });
}

function pumpLinks(): void {
  pumpScheduled = false;
  while (inFlight < MAX_IN_FLIGHT && queue.length > 0) {
    const job = queue.shift()!;
    if (!job.code.isConnected) continue;
    inFlight += 1;
    void lookup(job.root, job.query)
      .then((match) => {
        if (match) applyMatch(job.code, job.query, match);
      })
      .finally(() => {
        inFlight -= 1;
        if (queue.length > 0) schedulePump();
      });
  }
}

function schedulePump(): void {
  if (pumpScheduled || inFlight >= MAX_IN_FLIGHT) return;
  pumpScheduled = true;
  const run = () => pumpLinks();
  if (typeof window.requestIdleCallback === "function") {
    window.requestIdleCallback(run, { timeout: 400 });
  } else {
    window.setTimeout(run, 32);
  }
}

/** File names in a reply become links once the file is found in the project. */
export function linkFileMentions(container: HTMLElement): void {
  const project = rootOf().trim();
  const root = project && project !== "—" ? project : "/";
  for (const code of container.querySelectorAll<HTMLElement>("code")) {
    if (code.closest("pre, a, .chat-file-link")) continue;
    const query = code.textContent?.trim() ?? "";
    if (!looksLikeFileMention(query)) continue;
    if (root === "/" && !/^(?:\/|~\/)/.test(query)) continue;
    const cached = settled.get(cacheKey(root, query));
    if (cached) {
      applyMatch(code, query, cached);
      continue;
    }
    if (cached === null) continue;
    queue.push({ root, query, code });
  }
  if (queue.length > 0) schedulePump();
}
