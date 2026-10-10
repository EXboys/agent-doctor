import { t } from "../i18n";
import { rowActivityInfo, summarizeToolActivities, type ToolActivityInfo } from "./format";

/** Files the turn wrote or changed stay visible and clickable while the steps are folded. */
function changedFileChips(infos: ToolActivityInfo[]): HTMLElement | null {
  const byPath = new Map<string, ToolActivityInfo & { added: number; removed: number }>();
  for (const info of infos) {
    if ((info.kind !== "write" && info.kind !== "edit") || !info.path) continue;
    const seen = byPath.get(info.path);
    if (seen) {
      seen.added += info.additions;
      seen.removed += info.deletions;
      seen.addedLines = [...seen.addedLines, ...info.addedLines];
      seen.deletedLines = [...seen.deletedLines, ...info.deletedLines];
    } else {
      byPath.set(info.path, { ...info, added: info.additions, removed: info.deletions });
    }
  }
  if (byPath.size === 0) return null;
  const list = document.createElement("span");
  list.className = "chat-work-files";
  for (const file of byPath.values()) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = "chat-work-file";
    chip.title = file.path;
    const name = document.createElement("span");
    name.textContent = file.path.split(/[/\\]/).pop() || file.path;
    chip.append(name);
    if (file.added || file.removed) {
      const add = document.createElement("span");
      add.className = "chat-work-file-add";
      add.textContent = `+${file.added}`;
      const del = document.createElement("span");
      del.className = "chat-work-file-del";
      del.textContent = file.removed ? `−${file.removed}` : "";
      chip.append(add, del);
    }
    chip.addEventListener("click", (event) => {
      event.preventDefault();
      event.stopPropagation();
      window.dispatchEvent(
        new CustomEvent("chat-open-workspace-file", {
          detail: {
            path: file.path,
            additions: file.added,
            deletions: file.removed,
            addedLines: file.addedLines,
            deletedLines: file.deletedLines,
          },
        }),
      );
    });
    list.append(chip);
  }
  return list;
}

function completedTrail(rows: HTMLElement[]): HTMLDetailsElement {
  const trail = document.createElement("details");
  trail.className = "chat-work";
  trail.open = false;
  trail.innerHTML = `
    <summary class="chat-work-summary">
      <span class="chat-work-label"></span>
      <span class="chat-work-chevron" aria-hidden="true"></span>
    </summary>
    <div class="chat-work-body"></div>
  `;
  const toolRows = rows.flatMap((row) => [
    ...(row.matches(".chat-tool-row") ? [row] : []),
    ...row.querySelectorAll<HTMLElement>(".chat-tool-row"),
  ]);
  const infos = toolRows.map(rowActivityInfo);
  trail.querySelector<HTMLElement>(".chat-work-label")!.textContent =
    summarizeToolActivities(infos) || t("chat.workDone");
  const files = changedFileChips(infos);
  if (files) trail.querySelector(".chat-work-summary")!.append(files);
  const body = trail.querySelector<HTMLElement>(".chat-work-body")!;
  for (const row of rows) {
    if (row.classList.contains("chat-work-stream")) body.append(...Array.from(row.children));
    else body.append(row);
  }
  return trail;
}

export function wrapProcessRows(rows: HTMLElement[], live = false): HTMLElement {
  if (!live) return completedTrail(rows);
  if (rows.length === 1) return rows[0];
  const stream = document.createElement("div");
  stream.className = "chat-work-stream is-live";
  stream.append(...rows);
  return stream;
}

/** Add each process block directly to the transcript, in chronological order. */
export function placeProcessBlock(logEl: HTMLElement, node: HTMLElement): void {
  logEl.appendChild(node);
}

function isProcessBlock(el: HTMLElement): boolean {
  if (el.classList.contains("chat-thinking") || el.classList.contains("chat-work-stream")) {
    return true;
  }
  return (
    el.classList.contains("chat-tool-group") &&
    !el.classList.contains("chat-permission-group") &&
    !el.classList.contains("chat-turn-tools")
  );
}

export function sealWorkTrail(logEl: HTMLElement): void {
  const rows: HTMLElement[] = [];
  let current = logEl.lastElementChild as HTMLElement | null;
  while (current && isProcessBlock(current)) {
    rows.unshift(current);
    current = current.previousElementSibling as HTMLElement | null;
  }
  if (rows.length === 0) return;
  const marker = document.createComment("completed work");
  rows[0].before(marker);
  marker.replaceWith(completedTrail(rows));
}

/** Restored timelines retain their live marker while thinking continues. */
export function noteThinkingLive(block: HTMLElement): void {
  block.closest<HTMLElement>(".chat-work-stream")?.classList.add("is-live");
  const completed = block.closest<HTMLDetailsElement>(".chat-work");
  if (!completed) return;
  completed.classList.add("is-live");
  completed.open = true;
  const label = completed.querySelector<HTMLElement>(".chat-work-label");
  if (label) label.textContent = t("chat.workLive");
}
