import { convertFileSrc } from "@tauri-apps/api/core";
import { escapeHtml, renderMarkdown } from "../markdown";
import { t } from "../i18n";
import { highlightCode } from "./code-highlight";
import {
  findWorkspacePath,
  listWorkspaceDir,
  readWorkspaceFile,
  writeWorkspaceFile,
  type WorkspaceDirEntry,
  type WorkspaceSheet,
} from "../ipc";
import { fileChangeFor, hasFileChange, type FileChangePreview } from "./file-changes";
import { inlineFileDiff, type DiffLine } from "./file-diff";
import { pageAddress, wrapHtmlForPreview, type UiStep, type UiStepKind } from "./page-preview";
import { syncChatFilesSplitter } from "./layout-resize";

export type FilesPanelDeps = {
  mainEl: HTMLElement;
  toggleEl: HTMLButtonElement;
  panelEl: HTMLElement;
  listEl: HTMLElement;
  viewEl: HTMLElement;
  closeEl: HTMLButtonElement;
  layoutEl: HTMLButtonElement;
  treeEl: HTMLButtonElement;
  breadcrumbEl: HTMLElement;
  fileNameEl: HTMLElement;
  languageEl: HTMLElement;
  placeholderEl: HTMLElement;
  previewEl: HTMLPreElement;
  editorEl: HTMLTextAreaElement;
  codeEl: HTMLElement;
  imageEl: HTMLElement;
  sheetEl: HTMLElement;
  noticeEl: HTMLElement;
  saveEl: HTMLButtonElement;
  emptyEl: HTMLElement;
  getRoot: () => string;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
};

type PanelMode = "closed" | "list" | "file";
type FilesLayout = "cover" | "split";

const LAYOUT_KEY = "agent-doctor-ask-files-layout";
const TREE_KEY = "agent-doctor-ask-files-tree";

function readFilesLayout(): FilesLayout {
  return localStorage.getItem(LAYOUT_KEY) === "split" ? "split" : "cover";
}

export function createFilesPanel(deps: FilesPanelDeps) {
  let mode: PanelMode = "closed";
  let layout: FilesLayout = readFilesLayout();
  let treeCollapsed = localStorage.getItem(TREE_KEY) === "hidden";
  let listRelative = "";
  let openFileRelative = "";
  let openFileLanguage = "text";
  let dirty = false;
  const diffEl = deps.panelEl.querySelector<HTMLElement>("#chat-files-diff");
  const modeEl = deps.panelEl.querySelector<HTMLElement>("#chat-files-mode");
  const modePreviewEl = deps.panelEl.querySelector<HTMLButtonElement>("#chat-files-mode-preview");
  const modeSourceEl = deps.panelEl.querySelector<HTMLButtonElement>("#chat-files-mode-source");
  const renderedEl = deps.panelEl.querySelector<HTMLElement>("#chat-files-rendered");
  const renderedDocEl = deps.panelEl.querySelector<HTMLElement>(".chat-files-rendered-doc");
  const renderedFrameEl = deps.panelEl.querySelector<HTMLIFrameElement>(".chat-files-rendered-frame");
  const browserEl = deps.panelEl.querySelector<HTMLElement>("#chat-files-browser");
  const browserBackEl = deps.panelEl.querySelector<HTMLButtonElement>("#chat-files-browser-back");
  const browserForwardEl = deps.panelEl.querySelector<HTMLButtonElement>("#chat-files-browser-forward");
  const browserReloadEl = deps.panelEl.querySelector<HTMLButtonElement>("#chat-files-browser-reload");
  const browserAddressEl = deps.panelEl.querySelector<HTMLInputElement>("#chat-files-browser-address");
  const uiEl = deps.panelEl.querySelector<HTMLElement>("#chat-files-ui");
  const uiStepsEl = deps.panelEl.querySelector<HTMLElement>("#chat-files-ui-steps");
  const uiAddEl = deps.panelEl.querySelector<HTMLButtonElement>("#chat-files-ui-add");
  const uiRunEl = deps.panelEl.querySelector<HTMLButtonElement>("#chat-files-ui-run");
  const browserChecksEl = deps.panelEl.querySelector<HTMLElement>("#chat-files-browser-checks");
  let previewOn = false;
  let pageHtml = "";
  let pageStops: Array<{ kind: "file" } | { kind: "url"; url: string }> = [];
  let pageAt = -1;
  let uiSteps: UiStep[] = [{ kind: "click", target: "", text: "" }];
  let uiRunning = false;
  let uiResultWait = 0;
  let uiResultHandler: ((event: MessageEvent) => void) | null = null;
  /** Folder of a file opened from outside the project, such as /tmp. */
  let outsideRoot: string | null = null;

  function rootOrWarn(): string | null {
    if (outsideRoot) return outsideRoot;
    const root = deps.getRoot().trim();
    if (!root || root === "—") {
      deps.setStatus(t("chat.filesNoFolder"), "warn");
      return null;
    }
    return root;
  }

  function syncLayoutButton(): void {
    const split = layout === "split";
    const label = t(split ? "chat.filesCover" : "chat.filesSplit");
    const hint = t(split ? "chat.filesCoverHint" : "chat.filesSplitHint");
    deps.layoutEl.title = hint;
    deps.layoutEl.setAttribute("aria-label", label);
    deps.layoutEl.setAttribute("aria-pressed", split ? "true" : "false");
    deps.layoutEl.classList.toggle("is-on", split);
    deps.layoutEl.querySelector<SVGElement>(".chat-files-layout-icon-split")?.toggleAttribute("hidden", split);
    deps.layoutEl.querySelector<SVGElement>(".chat-files-layout-icon-cover")?.toggleAttribute("hidden", !split);
  }

  function syncTreeButton(): void {
    deps.panelEl.classList.toggle("is-tree-collapsed", treeCollapsed);
    const label = t(treeCollapsed ? "chat.filesTreeShow" : "chat.filesTreeHide");
    const hint = t(treeCollapsed ? "chat.filesTreeShowHint" : "chat.filesTreeHideHint");
    deps.treeEl.title = hint;
    deps.treeEl.setAttribute("aria-label", label);
    deps.treeEl.setAttribute("aria-pressed", treeCollapsed ? "true" : "false");
    const pick = deps.placeholderEl.querySelector<HTMLElement>("[data-i18n='chat.filesPickOne']");
    if (pick) pick.textContent = t(treeCollapsed ? "chat.filesPickOneFolded" : "chat.filesPickOne");
  }

  /** The corner button opens the list; a file in the chat opens the file with the list folded. */
  function rememberTree(collapsed: boolean): void {
    treeCollapsed = collapsed;
    localStorage.setItem(TREE_KEY, collapsed ? "hidden" : "shown");
  }

  function setOpen(open: boolean): void {
    mode = open ? "list" : "closed";
    deps.mainEl.classList.toggle("is-files-open", open);
    deps.mainEl.classList.toggle("is-files-split", open && layout === "split");
    deps.toggleEl.classList.toggle("is-on", open);
    deps.toggleEl.setAttribute("aria-pressed", open ? "true" : "false");
    deps.panelEl.hidden = !open;
    deps.panelEl.setAttribute("aria-hidden", open ? "false" : "true");
    syncChatFilesSplitter(open && layout === "split");
    syncLayoutButton();
    syncTreeButton();
  }

  function paintInlineRows(rows: DiffLine[]): void {
    const code = deps.previewEl.querySelector("code");
    if (!code) return;
    code.className = "";
    code.replaceChildren();
    let firstChange = -1;
    rows.forEach((row, index) => {
      const line = document.createElement("div");
      line.className = "chat-files-inline";
      line.dataset.kind = row.kind;
      line.textContent = row.text.length ? row.text : " ";
      code.append(line);
      if (firstChange < 0 && row.kind !== "same") firstChange = index;
    });
    if (firstChange > 2) {
      const lineHeight = parseFloat(getComputedStyle(deps.previewEl).lineHeight) || 22;
      deps.previewEl.scrollTop = Math.max(0, (firstChange - 2) * lineHeight);
    }
  }

  function canPreview(language: string): boolean {
    return language === "markdown" || language === "html";
  }

  function syncPreviewMode(): void {
    const show = canPreview(openFileLanguage) && !deps.viewEl.hidden && deps.placeholderEl.hidden;
    if (modeEl) modeEl.hidden = !show;
    modePreviewEl?.classList.toggle("is-on", previewOn);
    modeSourceEl?.classList.toggle("is-on", !previewOn);
    modePreviewEl?.setAttribute("aria-pressed", previewOn ? "true" : "false");
    modeSourceEl?.setAttribute("aria-pressed", previewOn ? "false" : "true");
    if (!show || !previewOn) {
      if (renderedEl) renderedEl.hidden = true;
      if (browserEl) browserEl.hidden = true;
      if (uiEl) uiEl.hidden = true;
      if (browserChecksEl) browserChecksEl.hidden = true;
      renderedFrameEl?.removeAttribute("srcdoc");
    }
  }

  function paintPageChecks(lines: Array<{ ok: boolean; text: string }> | null): void {
    if (!browserChecksEl) return;
    browserChecksEl.replaceChildren();
    if (!lines || lines.length === 0) {
      browserChecksEl.hidden = true;
      return;
    }
    browserChecksEl.hidden = false;
    for (const line of lines) {
      const item = document.createElement("li");
      item.dataset.ok = line.ok ? "1" : "0";
      item.textContent = line.text;
      browserChecksEl.append(item);
    }
  }

  function stepResultText(code: string, name: string): { ok: boolean; text: string } {
    const named = { name };
    if (code === "clicked") return { ok: true, text: t("chat.uiClicked", named) };
    if (code === "missing") return { ok: false, text: t("chat.uiMissing", named) };
    if (code === "filled") return { ok: true, text: t("chat.uiFilled", named) };
    if (code === "nofield") return { ok: false, text: t("chat.uiNoField", named) };
    if (code === "seen") return { ok: true, text: t("chat.uiSeen", named) };
    if (code === "unseen") return { ok: false, text: t("chat.uiUnseen", named) };
    return { ok: false, text: t("chat.uiFailed") };
  }

  function readUiStepsFromDom(): void {
    if (!uiStepsEl) return;
    const next: UiStep[] = [];
    uiStepsEl.querySelectorAll<HTMLElement>(".chat-files-ui-step").forEach((row) => {
      const kind = (row.querySelector("select")?.value || "click") as UiStepKind;
      const target = row.querySelector<HTMLInputElement>("[data-field='target']")?.value ?? "";
      const text = row.querySelector<HTMLInputElement>("[data-field='text']")?.value ?? "";
      next.push({ kind, target, text });
    });
    if (next.length) uiSteps = next;
  }

  function paintUiSteps(): void {
    if (!uiStepsEl) return;
    uiStepsEl.replaceChildren();
    if (uiSteps.length === 0) uiSteps = [{ kind: "click", target: "", text: "" }];
    for (const step of uiSteps) {
      const row = document.createElement("div");
      row.className = "chat-files-ui-step";
      const kind = document.createElement("select");
      kind.className = "chat-files-ui-kind";
      for (const optionKind of ["click", "fill", "see"] as const) {
        const option = document.createElement("option");
        option.value = optionKind;
        option.textContent = t(
          optionKind === "click" ? "chat.uiClick" : optionKind === "fill" ? "chat.uiFill" : "chat.uiSee",
        );
        if (optionKind === step.kind) option.selected = true;
        kind.append(option);
      }
      const target = document.createElement("input");
      target.className = "chat-files-ui-input";
      target.dataset.field = "target";
      target.value = step.target;
      const text = document.createElement("input");
      text.className = "chat-files-ui-input";
      text.dataset.field = "text";
      text.value = step.text;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "chat-files-ui-remove";
      remove.textContent = "×";
      remove.setAttribute("aria-label", t("chat.uiRemove"));
      const syncFields = () => {
        const current = kind.value as UiStepKind;
        text.hidden = current !== "fill";
        target.placeholder = t(
          current === "click" ? "chat.uiClickHint" : current === "fill" ? "chat.uiFieldHint" : "chat.uiSeeHint",
        );
        text.placeholder = t("chat.uiTextHint");
      };
      kind.addEventListener("change", () => {
        syncFields();
        readUiStepsFromDom();
      });
      target.addEventListener("input", () => readUiStepsFromDom());
      text.addEventListener("input", () => readUiStepsFromDom());
      remove.addEventListener("click", () => {
        readUiStepsFromDom();
        const index = Array.prototype.indexOf.call(uiStepsEl.children, row);
        uiSteps.splice(index, 1);
        paintUiSteps();
      });
      syncFields();
      row.append(kind, target, text, remove);
      uiStepsEl.append(row);
    }
  }

  function syncBrowserButtons(): void {
    if (browserBackEl) browserBackEl.disabled = pageAt <= 0;
    if (browserForwardEl) browserForwardEl.disabled = pageAt < 0 || pageAt >= pageStops.length - 1;
    const stop = pageStops[pageAt];
    if (browserAddressEl && document.activeElement !== browserAddressEl) {
      browserAddressEl.value = stop?.kind === "url" ? stop.url : deps.fileNameEl.textContent?.trim() || "";
    }
  }

  function loadPageStop(): void {
    if (!renderedFrameEl) return;
    const stop = pageStops[pageAt];
    paintPageChecks(null);
    window.clearTimeout(uiResultWait);
    if (!stop) return;
    if (stop.kind === "file") {
      renderedFrameEl.removeAttribute("src");
      renderedFrameEl.srcdoc = wrapHtmlForPreview(pageHtml);
    } else {
      renderedFrameEl.removeAttribute("srcdoc");
      renderedFrameEl.src = stop.url;
    }
    syncBrowserButtons();
  }

  function openHtmlPage(content: string): void {
    pageHtml = content;
    pageStops = [{ kind: "file" }];
    pageAt = 0;
    if (browserEl) browserEl.hidden = false;
    if (uiEl) uiEl.hidden = false;
    paintUiSteps();
    loadPageStop();
  }

  function askPageStep(step: UiStep, id: number): Promise<{ ok: boolean; code: string; target: string }> {
    const frame = renderedFrameEl?.contentWindow;
    if (!frame) return Promise.resolve({ ok: false, code: "failed", target: step.target });
    return new Promise((resolve) => {
      const finish = (code: string, ok: boolean) => {
        window.clearTimeout(uiResultWait);
        if (uiResultHandler) window.removeEventListener("message", uiResultHandler);
        uiResultHandler = null;
        resolve({ ok, code, target: step.target });
      };
      uiResultHandler = (event: MessageEvent) => {
        const data = event.data as { source?: string; id?: number; ok?: boolean; code?: string; target?: string } | null;
        if (!data || data.source !== "agent-doctor-ui-result" || data.id !== id) return;
        if (event.source !== frame) return;
        finish(data.code || "failed", Boolean(data.ok));
      };
      window.addEventListener("message", uiResultHandler);
      uiResultWait = window.setTimeout(() => finish("failed", false), 2000);
      frame.postMessage({ source: "agent-doctor-ui", id, step }, "*");
    });
  }

  async function runUiSteps(): Promise<void> {
    if (uiRunning) return;
    readUiStepsFromDom();
    const steps = uiSteps.filter((step) => step.target.trim());
    if (steps.length === 0) {
      deps.setStatus(t("chat.uiNeedStep"), "warn");
      return;
    }
    if (pageStops[pageAt]?.kind !== "file") {
      paintPageChecks([{ ok: false, text: t("chat.uiOutside") }]);
      return;
    }
    uiRunning = true;
    if (uiRunEl) uiRunEl.disabled = true;
    const lines: Array<{ ok: boolean; text: string }> = [];
    paintPageChecks([{ ok: true, text: t("chat.browserWaiting") }]);
    for (let index = 0; index < steps.length; index += 1) {
      const result = await askPageStep(steps[index], index + 1);
      lines.push(stepResultText(result.code, result.target.trim()));
      paintPageChecks(lines);
      if (!result.ok) break;
      await new Promise((resolve) => window.setTimeout(resolve, 180));
    }
    uiRunning = false;
    if (uiRunEl) uiRunEl.disabled = false;
  }

  function showRendered(content: string): void {
    if (!renderedEl || !renderedDocEl || !renderedFrameEl) return;
    previewOn = true;
    deps.codeEl.hidden = true;
    renderedEl.hidden = false;
    const html = openFileLanguage === "html";
    renderedEl.classList.toggle("is-html", html);
    renderedDocEl.hidden = html;
    renderedFrameEl.hidden = !html;
    if (html) {
      renderedDocEl.replaceChildren();
      renderedFrameEl.hidden = false;
      openHtmlPage(content);
    } else {
      if (browserEl) browserEl.hidden = true;
      paintPageChecks(null);
      renderedFrameEl.removeAttribute("srcdoc");
      renderedDocEl.innerHTML = renderMarkdown(content);
    }
    syncPreviewMode();
  }

  function showSource(): void {
    previewOn = false;
    if (renderedEl) renderedEl.hidden = true;
    renderedFrameEl?.removeAttribute("srcdoc");
    if (!deps.imageEl.hidden || !deps.sheetEl.hidden || !deps.noticeEl.hidden) {
      deps.codeEl.hidden = true;
    } else if (openFileLanguage) {
      deps.codeEl.hidden = false;
    }
    syncPreviewMode();
  }

  function showEditableFile(content: string, language: string): void {
    deps.codeEl.classList.remove("is-inline-diff");
    deps.editorEl.hidden = false;
    const code = deps.previewEl.querySelector("code");
    if (code) {
      code.className = `language-${language}`;
      code.innerHTML = highlightCode(content, language);
    }
  }

  function paintFile(
    content: string,
    language: string,
    editable: boolean,
    resetScroll = false,
    change?: FileChangePreview,
  ): void {
    const top = deps.editorEl.scrollTop;
    const left = deps.editorEl.scrollLeft;
    deps.editorEl.value = content;
    deps.editorEl.className = `chat-files-editor language-${language}`;
    const rows = editable && change && !dirty
      ? inlineFileDiff(content, change.deletedLines ?? [], change.addedLines ?? [])
      : null;
    if (rows) {
      paintInlineRows(rows);
      deps.codeEl.classList.add("is-inline-diff");
      deps.editorEl.hidden = true;
      if (diffEl) diffEl.hidden = true;
    } else {
      showEditableFile(content, language);
      deps.editorEl.hidden = !editable;
    }
    deps.saveEl.hidden = !editable;
    deps.saveEl.classList.remove("is-dirty");
    if (!rows) {
      deps.editorEl.scrollTop = resetScroll ? 0 : top;
      deps.editorEl.scrollLeft = resetScroll ? 0 : left;
      deps.previewEl.scrollTop = deps.editorEl.scrollTop;
      deps.previewEl.scrollLeft = deps.editorEl.scrollLeft;
    }
    if (previewOn && canPreview(language)) showRendered(content);
    else syncPreviewMode();
  }

  function columnLabel(index: number): string {
    let label = "";
    for (let n = index + 1; n > 0; n = Math.floor((n - 1) / 26)) {
      label = String.fromCharCode(65 + ((n - 1) % 26)) + label;
    }
    return label;
  }

  function paintSheet(sheets: WorkspaceSheet[], active: number): void {
    const tabs = deps.sheetEl.querySelector<HTMLElement>(".chat-files-sheet-tabs")!;
    const table = deps.sheetEl.querySelector<HTMLTableElement>(".chat-files-sheet-table")!;
    const note = deps.sheetEl.querySelector<HTMLElement>(".chat-files-sheet-note")!;
    const scroll = deps.sheetEl.querySelector<HTMLElement>(".chat-files-sheet-scroll")!;

    tabs.replaceChildren();
    tabs.hidden = sheets.length < 2;
    sheets.forEach((sheet, index) => {
      const tab = document.createElement("button");
      tab.type = "button";
      tab.className = `chat-files-sheet-tab${index === active ? " is-active" : ""}`;
      tab.setAttribute("role", "tab");
      tab.setAttribute("aria-selected", index === active ? "true" : "false");
      tab.textContent = sheet.name;
      tab.addEventListener("click", () => paintSheet(sheets, index));
      tabs.append(tab);
    });

    const sheet = sheets[active];
    const width = sheet.rows.reduce((max, row) => Math.max(max, row.length), 0);
    table.replaceChildren();
    if (width > 0) {
      const head = table.createTHead().insertRow();
      head.append(document.createElement("th"));
      for (let col = 0; col < width; col++) {
        const th = document.createElement("th");
        th.textContent = columnLabel(col);
        head.append(th);
      }
      const body = table.createTBody();
      sheet.rows.forEach((row, rowIndex) => {
        const tr = body.insertRow();
        const th = document.createElement("th");
        th.textContent = String(rowIndex + 1);
        tr.append(th);
        for (let col = 0; col < width; col++) {
          tr.insertCell().textContent = row[col] ?? "";
        }
      });
    }
    note.hidden = width > 0 && !sheet.truncated;
    note.textContent = width === 0 ? t("chat.filesSheetEmpty") : t("chat.filesSheetTruncated");
    scroll.scrollTop = 0;
    scroll.scrollLeft = 0;
  }

  function showList(): void {
    deps.listEl.hidden = false;
    void renderList();
  }

  function breadcrumbLabel(relative: string): string {
    if (!relative) return t("chat.filesHere");
    return relative.split("/").filter(Boolean).join(" / ");
  }

  async function renderList(): Promise<void> {
    const root = rootOrWarn();
    if (!root) {
      setOpen(false);
      return;
    }
    deps.breadcrumbEl.textContent = breadcrumbLabel(listRelative);
    for (const item of deps.listEl.querySelectorAll(".chat-files-item")) item.remove();
    deps.emptyEl.hidden = true;

    try {
      const entries = await listWorkspaceDir({ root, relative: listRelative || null });
      if (listRelative) {
        const up = document.createElement("button");
        up.type = "button";
        up.className = "chat-files-item is-dir";
        up.innerHTML = `<span class="chat-files-item-icon" aria-hidden="true">↩</span><span class="chat-files-item-name">${escapeHtml(t("chat.filesUp"))}</span>`;
        up.addEventListener("click", () => {
          const parts = listRelative.split("/").filter(Boolean);
          parts.pop();
          listRelative = parts.join("/");
          showList();
        });
        deps.listEl.append(up);
      }
      if (entries.length === 0) {
        deps.emptyEl.hidden = false;
        deps.emptyEl.textContent = t("chat.filesEmpty");
        return;
      }
      for (const entry of entries) {
        deps.listEl.append(renderEntry(entry));
      }
    } catch (error) {
      deps.setStatus(String(error ?? t("chat.filesLoadFailed")), "error");
    }
  }

  function renderEntry(entry: WorkspaceDirEntry): HTMLElement {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `chat-files-item${entry.isDir ? " is-dir" : " is-file"}`;
    btn.dataset.path = entry.relativePath;
    if (!entry.isDir && entry.relativePath === openFileRelative) {
      btn.classList.add("is-active");
    }
    const icon = entry.isDir
      ? `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M3.75 6.75A1.75 1.75 0 0 1 5.5 5h4.1l2 2.25h6.9A1.75 1.75 0 0 1 20.25 9v8A1.75 1.75 0 0 1 18.5 18.75h-13A1.75 1.75 0 0 1 3.75 17V6.75Z" stroke="currentColor" stroke-width="1.55" stroke-linejoin="round"/></svg>`
      : `<svg width="16" height="16" viewBox="0 0 24 24" fill="none"><path d="M6 3.75h7l5 5V20.25H6V3.75Z" stroke="currentColor" stroke-width="1.45" stroke-linejoin="round"/><path d="M13 3.75v5h5" stroke="currentColor" stroke-width="1.45" stroke-linejoin="round"/></svg>`;
    btn.innerHTML = `<span class="chat-files-item-icon" aria-hidden="true">${icon}</span><span class="chat-files-item-name">${escapeHtml(entry.name)}</span>${entry.isDir ? `<span class="chat-files-item-chevron" aria-hidden="true">›</span>` : ""}`;
    btn.addEventListener("click", () => {
      if (entry.isDir) {
        listRelative = entry.relativePath;
        showList();
        return;
      }
      void openFile(entry.relativePath, btn);
    });
    return btn;
  }

  function revealLines(content: string, start = 0, end = start): void {
    if (start < 1) return;
    const lines = content.split("\n");
    const first = Math.min(start, lines.length);
    const last = Math.min(Math.max(end || first, first), lines.length);
    let selectionStart = 0;
    for (let i = 0; i < first - 1; i++) selectionStart += lines[i].length + 1;
    let selectionEnd = selectionStart;
    for (let i = first - 1; i < last; i++) selectionEnd += lines[i].length + (i < last - 1 ? 1 : 0);
    deps.editorEl.setSelectionRange(selectionStart, selectionEnd);
    const lineHeight = parseFloat(getComputedStyle(deps.editorEl).lineHeight) || 20;
    deps.editorEl.scrollTop = Math.max(0, (first - 2) * lineHeight);
    deps.previewEl.scrollTop = deps.editorEl.scrollTop;
  }

  async function openFile(
    relative: string,
    row?: HTMLElement,
    change?: FileChangePreview,
  ): Promise<void> {
    const root = rootOrWarn();
    if (!root) return;
    try {
      const file = await readWorkspaceFile({ root, relative });
      openFileRelative = file.relativePath;
      openFileLanguage = file.language;
      previewOn = false;
      dirty = false;
      deps.viewEl.hidden = false;
      deps.placeholderEl.hidden = true;
      deps.codeEl.hidden = !file.editable;
      deps.imageEl.hidden = !file.isImage;
      deps.sheetEl.hidden = !file.sheets;
      if (file.sheets) paintSheet(file.sheets, 0);
      deps.noticeEl.hidden = file.editable || file.isImage || Boolean(file.sheets);
      deps.noticeEl.textContent = t("chat.filesBinary");
      const img = deps.imageEl.querySelector("img");
      if (img) {
        if (file.isImage) {
          img.alt = file.name;
          img.src = convertFileSrc(file.absolutePath);
        } else {
          img.removeAttribute("src");
        }
      }
      deps.fileNameEl.textContent = file.name;
      deps.languageEl.hidden = !file.editable;
      deps.languageEl.textContent = file.language.toUpperCase();
      deps.breadcrumbEl.textContent = file.relativePath;
      const shown = hasFileChange(change) ? change : fileChangeFor(file.relativePath);
      if (diffEl) diffEl.hidden = true;
      deps.listEl.querySelectorAll(".chat-files-item.is-active").forEach((item) => {
        item.classList.remove("is-active");
      });
      row?.classList.add("is-active");
      paintFile(file.content, file.language, file.editable, true, shown);
      if (file.editable && !deps.codeEl.classList.contains("is-inline-diff")) {
        deps.editorEl.focus();
        revealLines(file.content, shown?.lineStart, shown?.lineEnd);
      }
    } catch (error) {
      deps.setStatus(String(error ?? t("chat.filesLoadFailed")), "error");
    }
  }

  async function saveFile(): Promise<void> {
    const root = rootOrWarn();
    if (!root || !openFileRelative) return;
    try {
      await writeWorkspaceFile({
        root,
        relative: openFileRelative,
        content: deps.editorEl.value,
      });
      dirty = false;
      deps.saveEl.classList.remove("is-dirty");
      paintFile(deps.editorEl.value, openFileLanguage, true);
      deps.setStatus(t("chat.filesSaved"), "ok");
    } catch (error) {
      deps.setStatus(String(error ?? t("chat.filesSaveFailed")), "error");
    }
  }

  deps.toggleEl.addEventListener("click", () => {
    if (mode !== "closed") {
      setOpen(false);
      return;
    }
    if (outsideRoot) {
      outsideRoot = null;
      listRelative = "";
    }
    rememberTree(false);
    setOpen(true);
    showList();
  });

  deps.closeEl.addEventListener("click", () => setOpen(false));
  deps.layoutEl.addEventListener("click", () => {
    layout = layout === "split" ? "cover" : "split";
    localStorage.setItem(LAYOUT_KEY, layout);
    if (mode !== "closed") setOpen(true);
    else syncLayoutButton();
  });
  deps.treeEl.addEventListener("click", () => {
    rememberTree(!treeCollapsed);
    syncTreeButton();
  });

  window.addEventListener("chat-open-workspace-file", (event) => {
    const detail = (event as CustomEvent<Partial<FileChangePreview> & { path?: string }>).detail;
    const projectRoot = deps.getRoot().trim();
    const raw = detail?.path?.trim();
    if (!raw) return;
    const root = projectRoot && projectRoot !== "—" ? projectRoot : "/";
    void findWorkspacePath({ root, query: raw })
      .catch(() => null)
      .then((match) => {
        if (!match) {
          deps.setStatus(t("chat.filesNotFound"), "warn");
          return;
        }
        let relative = match.relativePath;
        if (match.external) {
          const cut = relative.lastIndexOf("/");
          outsideRoot = match.isDir ? relative : relative.slice(0, cut) || "/";
          relative = match.isDir ? "" : relative.slice(cut + 1);
        } else {
          outsideRoot = null;
        }
        layout = "split";
        localStorage.setItem(LAYOUT_KEY, layout);
        rememberTree(true);
        setOpen(true);
        if (match.isDir) {
          listRelative = relative;
          showList();
          return;
        }
        void openFile(relative, undefined, {
          additions: detail.additions ?? 0,
          deletions: detail.deletions ?? 0,
          addedLines: detail.addedLines ?? [],
          deletedLines: detail.deletedLines ?? [],
          lineStart: detail.lineStart,
          lineEnd: detail.lineEnd,
        });
      });
  });

  browserBackEl?.addEventListener("click", () => {
    if (pageAt <= 0) return;
    pageAt -= 1;
    loadPageStop();
  });
  browserForwardEl?.addEventListener("click", () => {
    if (pageAt >= pageStops.length - 1) return;
    pageAt += 1;
    loadPageStop();
  });
  browserReloadEl?.addEventListener("click", () => {
    const stop = pageStops[pageAt];
    if (!stop) return;
    if (stop.kind === "file") pageHtml = deps.editorEl.value;
    loadPageStop();
  });
  browserAddressEl?.addEventListener("keydown", (event) => {
    if (event.key !== "Enter") return;
    const url = pageAddress(browserAddressEl.value);
    if (!url) {
      deps.setStatus(t("chat.browserBadAddress"), "warn");
      return;
    }
    pageStops = pageStops.slice(0, pageAt + 1);
    pageStops.push({ kind: "url", url });
    pageAt = pageStops.length - 1;
    loadPageStop();
  });
  uiAddEl?.addEventListener("click", () => {
    readUiStepsFromDom();
    uiSteps.push({ kind: "see", target: "", text: "" });
    paintUiSteps();
  });
  uiRunEl?.addEventListener("click", () => {
    void runUiSteps();
  });
  modePreviewEl?.addEventListener("click", () => {
    if (!canPreview(openFileLanguage) || previewOn) return;
    showRendered(deps.editorEl.value);
  });
  modeSourceEl?.addEventListener("click", () => {
    if (!previewOn) return;
    showSource();
  });
  deps.previewEl.addEventListener("click", () => {
    if (!deps.codeEl.classList.contains("is-inline-diff")) return;
    showEditableFile(deps.editorEl.value, openFileLanguage);
    deps.editorEl.focus();
  });
  deps.saveEl.addEventListener("click", () => void saveFile());
  deps.editorEl.addEventListener("scroll", () => {
    deps.previewEl.scrollTop = deps.editorEl.scrollTop;
    deps.previewEl.scrollLeft = deps.editorEl.scrollLeft;
  });
  deps.editorEl.addEventListener("input", () => {
    dirty = true;
    deps.saveEl.classList.add("is-dirty");
    const code = deps.previewEl.querySelector("code");
    if (code) code.innerHTML = highlightCode(deps.editorEl.value, openFileLanguage);
  });
  deps.editorEl.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
      event.preventDefault();
      void saveFile();
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      const start = deps.editorEl.selectionStart;
      const end = deps.editorEl.selectionEnd;
      deps.editorEl.setRangeText("  ", start, end, "end");
      deps.editorEl.dispatchEvent(new Event("input", { bubbles: true }));
    }
  });

  return {
    applyI18n() {
      deps.toggleEl.title = t("chat.filesToggleHint");
      deps.toggleEl.setAttribute("aria-label", t("chat.filesToggle"));
      deps.closeEl.textContent = t("chat.filesClose");
      deps.saveEl.textContent = t("chat.filesSave");
      if (modeEl) modeEl.setAttribute("aria-label", t("chat.filesPreview"));
      if (modePreviewEl) modePreviewEl.textContent = t("chat.filesPreview");
      if (modeSourceEl) modeSourceEl.textContent = t("chat.filesSource");
      if (renderedFrameEl) renderedFrameEl.title = t("chat.filesPreview");
      if (browserBackEl) browserBackEl.setAttribute("aria-label", t("chat.browserBack"));
      if (browserForwardEl) browserForwardEl.setAttribute("aria-label", t("chat.browserForward"));
      if (browserReloadEl) browserReloadEl.setAttribute("aria-label", t("chat.browserReload"));
      if (browserAddressEl) browserAddressEl.placeholder = t("chat.browserAddress");
      if (uiAddEl) uiAddEl.textContent = t("chat.uiAdd");
      if (uiRunEl) uiRunEl.textContent = t("chat.uiRun");
      if (uiEl && !uiEl.hidden) paintUiSteps();
      syncLayoutButton();
      syncTreeButton();
    },
    isOpen: () => mode !== "closed",
    close: () => setOpen(false),
    isDirty: () => dirty,
  };
}
