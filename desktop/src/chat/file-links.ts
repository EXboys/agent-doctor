import { findWorkspacePath, type WorkspaceMatch } from "../ipc";

let rootOf: () => string = () => "";
const known = new Map<string, Promise<WorkspaceMatch | null>>();

export function setFileLinkRoot(getRoot: () => string): void {
  rootOf = getRoot;
}

export function fileLinkRoot(): string {
  return rootOf().trim();
}

/** `digest.mjs`, `logs/a.md`, `.daily-digest`, `~/x/y.json` — not `0 9 1-5` or `1.0.31`. */
export function looksLikeFileMention(text: string): boolean {
  const value = text.trim();
  if (!value || value.length > 240 || /[\n<>|*?`]/.test(value)) return false;
  if (/^https?:\/\//i.test(value)) return false;
  const last = value.replace(/\/+$/, "").split("/").pop() ?? "";
  const hasExtension = /\.[a-z][a-z0-9]{0,9}$/i.test(last);
  const dotFolder = /^\.[a-z0-9][\w.-]*$/i.test(last);
  const pathLike = value.includes("/") && !/\s/.test(value);
  return hasExtension || dotFolder || pathLike;
}

function lookup(root: string, query: string): Promise<WorkspaceMatch | null> {
  const key = `${root}\u0001${query}`;
  let found = known.get(key);
  if (!found) {
    found = findWorkspacePath({ root, query }).catch(() => null);
    known.set(key, found);
  }
  return found;
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
    void lookup(root, query).then((match) => {
      if (!match || !code.isConnected || code.classList.contains("chat-file-link")) return;
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
    });
  }
}
