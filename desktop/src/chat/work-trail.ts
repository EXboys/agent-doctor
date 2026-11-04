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

export function completedTrail(rows: HTMLElement[]): HTMLDetailsElement {
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
    // Re-seals and history repaints can hand back earlier trails: flatten them
    // instead of nesting a trail inside a trail.
    if (row.classList.contains("chat-work-stream")) body.append(...Array.from(row.children));
    else if (row.classList.contains("chat-work")) {
      const inner = row.querySelector<HTMLElement>(".chat-work-body");
      if (inner) body.append(...Array.from(inner.children));
      else body.append(row);
    } else body.append(row);
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

/**
 * Fold one finished group into a labelled trail, in place. Turn end cannot
 * reach every group through the trailing walk (the reply bubble stops it), so
 * the group folds the moment it finishes instead of staying expanded until a
 * session switch re-renders the timeline.
 */
export function sealWorkGroup(group: HTMLDetailsElement): void {
  if (!group.isConnected || !group.querySelector(".chat-tool-row")) return;
  // completedTrail moves the group into the trail's body; replacing the group
  // afterwards would swap a node with its own ancestor (HierarchyRequestError,
  // which silently aborted every fold). Park a marker at the group's position
  // first, same as sealWorkTrail.
  const marker = document.createComment("finished work");
  group.before(marker);
  marker.replaceWith(completedTrail([group]));
}

/** Add each process block directly to the transcript, in chronological order. */
export function placeProcessBlock(logEl: HTMLElement, node: HTMLElement): void {
  logEl.appendChild(node);
}

/**
 * Fold the trailing tool groups into one labelled trail. Thinking blocks are
 * not process rows: they stay standalone between the chips and the prose. The
 * walk skips over them (a chatty agent like the built-in doctor interleaves a
 * think block after every tool round), and a trail made only of thinking is
 * not sealed at all — folding those stacked one empty-looking bar per round.
 */
/**
 * Fold one turn's process container (live stream of status rows, tool groups
 * and thinking blocks) into a collapsed, labelled trail — by reference, at
 * turn completion. Walking the log cannot do this: the streaming reply bubble
 * sits between the process rows and the end of the transcript and would stop
 * a trailing walk after the first line of reply text.
 */
export function sealProcessContainer(container: HTMLElement): void {
  console.error("S0 enter, children:", container.children.length);
  if (!container.isConnected) return;
  if (!container.firstElementChild) {
    container.remove();
    return;
  }
  console.error("S1 guard ok");
  const liveRows = container.querySelectorAll<HTMLElement>(
    ".chat-activity.is-live, .chat-tool-row.is-live",
  );
  console.error("S2 liveRows:", liveRows.length);
  for (const live of liveRows) {
    live.classList.remove("is-live");
    live.classList.add("is-done");
    live.querySelector(".chat-spinner")?.remove();
  }
  console.error("S3 settled");
  const marker = document.createComment("completed work");
  container.before(marker);
  console.error("S4 marker");
  const trail = completedTrail([container]);
  console.error("S5 trail built");
  marker.replaceWith(trail);
  console.error("S6 replaced");
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
