import { t, type MessageKey } from "../i18n";
import {
  isReadOnlyTerminalCommand,
  readRowStep,
  rowActivityInfo,
  summarizeToolActivities,
  type ToolActivityInfo,
} from "./format";

export type StageBucket = "explore" | "terminal" | "changes" | "browser";

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

/**
 * Classify a tool row into a ZCode-style stage bucket:
 * - `explore`: Read, Search, and read-only Shell inspection commands
 * - `terminal`: mutating / general Shell commands
 * - `changes`: Write / Edit
 * - `browser`: Computer-use / Browser surface actions
 * Subagent calls and unclassified generic rows stay standalone (`null`).
 */
export function classifyToolRowStage(row: HTMLElement): StageBucket | null {
  const info = rowActivityInfo(row);
  if (info.kind === "read" || info.kind === "search") return "explore";
  if (info.kind === "write" || info.kind === "edit") return "changes";
  if (info.kind === "browser") return "browser";
  if (info.kind === "subagent") return null;
  if (info.kind === "terminal") {
    const step = readRowStep(row);
    const command = step?.command || row.dataset.detail || "";
    if (!command.trim() && (step?.status === "running" || row.classList.contains("is-live"))) {
      // Unclassified in-flight shell without a command yet: keep standalone until command arrives.
      return null;
    }
    return isReadOnlyTerminalCommand(command) ? "explore" : "terminal";
  }
  return null;
}

function stageLiveLabel(stage: StageBucket): string {
  const keys: Record<StageBucket, MessageKey> = {
    explore: "chat.stageExploreLive",
    terminal: "chat.stageTerminalLive",
    changes: "chat.stageChangesLive",
    browser: "chat.stageBrowserLive",
  };
  return t(keys[stage]);
}

function rowLatestActionText(row: HTMLElement): string {
  const info = rowActivityInfo(row);
  if (info.target) return `${info.label} ${info.target}`.trim();
  return info.label;
}

function summarizeStageRows(stage: StageBucket, rows: HTMLElement[]): { text: string; infos: ToolActivityInfo[] } {
  const infos = rows.map(rowActivityInfo);
  if (stage === "explore") {
    const readFiles = new Set<string>();
    let searches = 0;
    let inspections = 0;
    for (const info of infos) {
      if (info.kind === "read") {
        readFiles.add(info.path || info.target || `read-${readFiles.size}`);
      } else if (info.kind === "search") {
        searches += 1;
      } else {
        inspections += 1;
      }
    }
    const parts: string[] = [];
    if (readFiles.size > 0) parts.push(t("chat.workReadFiles", { count: String(readFiles.size) }));
    if (searches > 0) parts.push(t("chat.workSearches", { count: String(searches) }));
    if (inspections > 0) parts.push(t("chat.workInspections", { count: String(inspections) }));
    return { text: parts.join(" · ") || summarizeToolActivities(infos), infos };
  }
  if (stage === "terminal") {
    const failed = rows.filter(
      (r) => r.classList.contains("is-failed") || readRowStep(r)?.status === "failed",
    ).length;
    const parts = [t("chat.workCommands", { count: String(rows.length) })];
    if (failed > 0) parts.push(t("chat.workFailedCount", { count: String(failed) }));
    return { text: parts.join(" · "), infos };
  }
  return { text: summarizeToolActivities(infos), infos };
}

export function createStageGroupElement(stage: StageBucket): HTMLDetailsElement {
  const group = document.createElement("details");
  group.className = `chat-stage-group is-${stage}`;
  group.dataset.stage = stage;
  group.open = false;
  group.innerHTML = `
    <summary class="chat-stage-summary">
      <span class="chat-stage-icon" aria-hidden="true"></span>
      <span class="chat-stage-label"></span>
      <span class="chat-stage-detail" hidden></span>
      <span class="chat-stage-chevron" aria-hidden="true"></span>
    </summary>
    <div class="chat-stage-body"></div>
  `;
  group.addEventListener("toggle", () => {
    if (group.dataset.syncing === "1") return;
    group.dataset.userToggled = "1";
    const live = group.classList.contains("is-live");
    paintStageSummary(group, live);
  });
  return group;
}

function setStageOpen(group: HTMLDetailsElement, open: boolean): void {
  if (group.open === open) return;
  group.dataset.syncing = "1";
  group.open = open;
  delete group.dataset.syncing;
}

export function paintStageSummary(group: HTMLDetailsElement, live: boolean): void {
  const stage = (group.dataset.stage as StageBucket | undefined) ?? "explore";
  const body = group.querySelector<HTMLElement>(":scope > .chat-stage-body");
  const rows = body ? [...body.querySelectorAll<HTMLElement>(":scope > .chat-tool-row")] : [];
  group.classList.toggle("is-live", live);
  if (group.dataset.userToggled !== "1") {
    setStageOpen(group, false);
  }
  const summaryEl = group.querySelector<HTMLElement>(":scope > .chat-stage-summary");
  const labelEl = group.querySelector<HTMLElement>(":scope > .chat-stage-summary > .chat-stage-label");
  const detailEl = group.querySelector<HTMLElement>(":scope > .chat-stage-summary > .chat-stage-detail");
  if (!summaryEl || !labelEl || !detailEl) return;

  summaryEl.querySelector(":scope > .chat-work-files")?.remove();
  const { text: completedText, infos } = summarizeStageRows(stage, rows);

  if (live && !group.open) {
    labelEl.textContent = stageLiveLabel(stage);
    const activeRow =
      [...rows].reverse().find((r) => r.classList.contains("is-live")) ?? rows[rows.length - 1];
    const latest = activeRow ? rowLatestActionText(activeRow) : "";
    detailEl.textContent = latest ? `· ${latest}` : "";
    detailEl.hidden = !latest;
    return;
  }

  labelEl.textContent = completedText || stageLiveLabel(stage);
  detailEl.textContent = "";
  detailEl.hidden = true;
  if (!group.open && stage === "changes") {
    const chips = changedFileChips(infos);
    if (chips) summaryEl.append(chips);
  }
}

function collectFlatProcessUnits(nodes: Iterable<Element>): HTMLElement[] {
  const out: HTMLElement[] = [];
  const visit = (node: Element) => {
    if (node.classList.contains("chat-work")) {
      const inner = node.querySelector<HTMLElement>(":scope > .chat-work-body");
      if (inner) {
        for (const child of Array.from(inner.children)) visit(child);
        return;
      }
    }
    if (node.classList.contains("chat-stage-group")) {
      const inner = node.querySelector<HTMLElement>(":scope > .chat-stage-body");
      if (inner) {
        for (const child of Array.from(inner.children)) visit(child);
        return;
      }
    }
    if (node.classList.contains("chat-work-stream")) {
      for (const child of Array.from(node.children)) visit(child);
      return;
    }
    if (node.classList.contains("chat-tool-group")) {
      const list = node.querySelector<HTMLElement>(":scope > .chat-tool-list");
      if (list) {
        for (const row of Array.from(list.children)) out.push(row as HTMLElement);
        return;
      }
    }
    if (node instanceof HTMLElement) out.push(node);
  };
  for (const node of nodes) visit(node);
  return out;
}

/**
 * Group consecutive compatible tool rows (>= 2) into stage rows (`Explore`,
 * `Terminal`, `Changes`, `Browser`) while keeping single tool rows and
 * `chat-thinking` blocks as flat siblings (ZCode / Paseo flow rhythm).
 */
export function buildStageFlowNodes(units: Iterable<Element>, tailRunning = false): HTMLElement[] {
  const flat = collectFlatProcessUnits(units);
  const out: HTMLElement[] = [];
  let i = 0;
  while (i < flat.length) {
    const item = flat[i]!;
    if (!item.classList.contains("chat-tool-row")) {
      out.push(item);
      i += 1;
      continue;
    }
    const stage = classifyToolRowStage(item);
    if (!stage) {
      out.push(item);
      i += 1;
      continue;
    }
    const run: HTMLElement[] = [item];
    while (i + run.length < flat.length) {
      const next = flat[i + run.length]!;
      if (!next.classList.contains("chat-tool-row")) break;
      if (classifyToolRowStage(next) !== stage) break;
      run.push(next);
    }
    i += run.length;
    // ZCode threshold: a single tool stays its own flat row; >= 2 forms a stage bar.
    if (run.length < 2) {
      out.push(run[0]!);
      continue;
    }
    const group = createStageGroupElement(stage);
    const body = group.querySelector<HTMLElement>(":scope > .chat-stage-body")!;
    body.append(...run);
    const isTailLive = tailRunning && i === flat.length;
    paintStageSummary(group, isTailLive);
    out.push(group);
  }
  return out;
}

/**
 * Settle any trailing live stage group inside `stream` once a new visible
 * boundary (thinking block, different tool stage, or assistant text) arrives.
 */
export function settleTrailingStageInStream(stream: HTMLElement | null | undefined): void {
  if (!stream) return;
  for (const group of stream.querySelectorAll<HTMLDetailsElement>(":scope > .chat-stage-group.is-live")) {
    paintStageSummary(group, false);
  }
}

/**
 * Append or update a live tool row inside `stream`, promoting 2+ consecutive
 * same-stage tool rows into a live collapsed stage bar in place so the
 * reader's manual expand choice survives streaming updates.
 */
export function appendToolRowToStageStream(stream: HTMLElement, row: HTMLElement): void {
  const stage = classifyToolRowStage(row);
  const last = stream.lastElementChild as HTMLElement | null;

  if (!stage) {
    settleTrailingStageInStream(stream);
    stream.appendChild(row);
    return;
  }

  if (last instanceof HTMLDetailsElement && last.classList.contains("chat-stage-group")) {
    if (last.dataset.stage === stage) {
      const body = last.querySelector<HTMLElement>(":scope > .chat-stage-body")!;
      body.appendChild(row);
      paintStageSummary(last, true);
      return;
    }
    paintStageSummary(last, false);
    stream.appendChild(row);
    return;
  }

  if (last && last.classList.contains("chat-tool-row")) {
    const prevStage = classifyToolRowStage(last);
    if (prevStage === stage) {
      const group = createStageGroupElement(stage);
      const marker = document.createComment("stage");
      last.before(marker);
      const body = group.querySelector<HTMLElement>(":scope > .chat-stage-body")!;
      body.append(last, row);
      paintStageSummary(group, true);
      marker.replaceWith(group);
      return;
    }
  }

  stream.appendChild(row);
}

/** Re-evaluate a tool row whose step just updated (e.g. shell command arrived or finished). */
export function refreshStageRowInStream(row: HTMLElement, tailRunning = true): void {
  const parentGroup = row.closest<HTMLDetailsElement>(".chat-stage-group");
  if (parentGroup) {
    const stream = parentGroup.parentElement;
    const isTail = Boolean(tailRunning && stream && stream.lastElementChild === parentGroup);
    paintStageSummary(parentGroup, isTail);
    return;
  }
  const stream = row.parentElement;
  if (!stream || !stream.classList.contains("chat-work-stream")) return;
  const stage = classifyToolRowStage(row);
  if (!stage) return;
  const prev = row.previousElementSibling as HTMLElement | null;
  if (prev instanceof HTMLDetailsElement && prev.classList.contains("chat-stage-group") && prev.dataset.stage === stage) {
    const body = prev.querySelector<HTMLElement>(":scope > .chat-stage-body")!;
    body.appendChild(row);
    paintStageSummary(prev, tailRunning && stream.lastElementChild === prev);
    return;
  }
  if (prev && prev.classList.contains("chat-tool-row") && classifyToolRowStage(prev) === stage) {
    const group = createStageGroupElement(stage);
    const marker = document.createComment("stage");
    prev.before(marker);
    const body = group.querySelector<HTMLElement>(":scope > .chat-stage-body")!;
    body.append(prev, row);
    marker.replaceWith(group);
    paintStageSummary(group, tailRunning && stream.lastElementChild === group);
  }
}

export function completedTrail(rows: HTMLElement[]): HTMLDetailsElement {
  const trail = document.createElement("details");
  trail.className = "chat-work";
  trail.open = false;
  trail.innerHTML = `
    <summary class="chat-work-summary">
      <span class="chat-work-icon" aria-hidden="true"></span>
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
  for (const unit of collectFlatProcessUnits(rows)) body.append(unit);
  return trail;
}

/**
 * Wrap a contiguous segment of process rows (thinking + tools between prose
 * paragraphs). Instead of collapsing an entire 40-step turn into one giant
 * opaque `查阅 6 个文件 · 其他 31 项` box, consecutive same-stage tools fold
 * into compact stage rows (`查阅 3 个文件 · 搜索 2 次`, `执行 4 条命令`) while
 * thinking rows and single tools stay in chronological rhythm.
 */
export function wrapProcessRows(rows: HTMLElement[], live = false): HTMLElement {
  const staged = buildStageFlowNodes(rows, live);
  if (!live && staged.length === 1) return staged[0]!;
  const stream = document.createElement("div");
  stream.className = live ? "chat-work-stream is-live" : "chat-work-stream";
  stream.append(...staged);
  return stream;
}

/** Fold one finished tool group into stage-grouped rows in place. */
export function sealWorkGroup(group: HTMLDetailsElement): void {
  if (!group.isConnected || !group.querySelector(".chat-tool-row")) return;
  const marker = document.createComment("finished work");
  group.before(marker);
  const staged = buildStageFlowNodes([group], false);
  marker.replaceWith(...staged);
  group.remove();
}

/** Add each process block directly to the transcript, in chronological order. */
export function placeProcessBlock(logEl: HTMLElement, node: HTMLElement): void {
  logEl.appendChild(node);
}

/**
 * Settle one process segment container (between assistant text paragraphs or
 * at turn completion) into clean stage-grouped rows without merging across
 * prose segments.
 */
export function sealProcessContainer(container: HTMLElement): void {
  if (!container.isConnected) return;
  for (const live of container.querySelectorAll<HTMLElement>(
    ".chat-activity.is-live, .chat-tool-row.is-live",
  )) {
    live.classList.remove("is-live");
    live.classList.add("is-done");
    live.querySelector(".chat-spinner")?.remove();
  }
  // Transient lifecycle spinners ("正在请求模型…") are ephemeral, not saved steps.
  for (const row of container.querySelectorAll<HTMLElement>(":scope > .chat-activity:not(.chat-tool-row)")) {
    const kind = row.dataset.kind ?? "";
    if (kind !== "error" && kind !== "log") row.remove();
  }
  container.querySelector(".chat-work-live-pill")?.remove();
  container.classList.remove("is-folded", "is-live");

  const staged = buildStageFlowNodes(Array.from(container.children), false);
  const marker = document.createComment("completed segment");
  container.before(marker);
  if (staged.length > 0) {
    marker.replaceWith(...staged);
  } else {
    marker.remove();
  }
  container.remove();
}

/** Restored timelines retain their live marker while thinking continues. */
export function noteThinkingLive(block: HTMLElement): void {
  block.closest<HTMLElement>(".chat-work-stream")?.classList.add("is-live");
}
