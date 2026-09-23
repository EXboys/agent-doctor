import { t } from "../i18n";
import {
  COMPACT_KEEP_TURNS,
  CONTEXT_RING_LENGTH,
  type ChatSession,
} from "./types";

export type ContextMeterEls = {
  contextMeterEl: HTMLButtonElement | null;
  contextRingFillEl: SVGCircleElement | null;
  contextLabelEl: HTMLElement | null;
  contextPopoverEl: HTMLElement | null;
  contextPopoverTitleEl: HTMLElement | null;
  contextPopoverBodyEl: HTMLElement | null;
  contextCompactEl: HTMLButtonElement | null;
  composerBoxEl: HTMLElement;
  composerEl: HTMLElement;
  promptEl: HTMLTextAreaElement;
};

export type ContextMeterDeps = ContextMeterEls & {
  activeSession: () => ChatSession;
  contextUsagePercent: (session: ChatSession, draft?: string) => number;
  isComposerLocked: () => boolean;
  closeModelMenu: () => void;
};

export type ContextMeterApi = ReturnType<typeof createContextMeterController>;

export function createContextMeterController(deps: ContextMeterDeps) {
  function closeContextPopover(): void {
    if (!deps.contextPopoverEl || !deps.contextMeterEl) return;
    deps.contextPopoverEl.hidden = true;
    deps.contextPopoverEl.classList.remove("is-open");
    deps.contextMeterEl.classList.remove("is-open");
    deps.contextMeterEl.setAttribute("aria-expanded", "false");
    deps.contextMeterEl.closest(".chat-context-wrap")?.classList.remove("is-open");
    deps.composerBoxEl.classList.remove("is-context-open");
    deps.composerEl.classList.remove("is-context-open");
    deps.contextPopoverEl.style.left = "";
    deps.contextPopoverEl.style.right = "";
    deps.contextPopoverEl.style.top = "";
    deps.contextPopoverEl.style.bottom = "";
    deps.contextPopoverEl.style.width = "";
  }

  function positionContextPopover(): void {
    if (!deps.contextPopoverEl || !deps.contextMeterEl || deps.contextPopoverEl.hidden) return;
    const rect = deps.contextMeterEl.getBoundingClientRect();
    const gap = 8;
    const width = Math.min(260, window.innerWidth - 24);
    const popH = Math.max(deps.contextPopoverEl.offsetHeight, 120);
    let left = rect.right - width;
    if (left < 12) left = 12;
    if (left + width > window.innerWidth - 12) {
      left = Math.max(12, window.innerWidth - 12 - width);
    }
    let top = rect.top - gap - popH;
    if (top < 12) {
      top = Math.min(rect.bottom + gap, window.innerHeight - popH - 12);
    }
    deps.contextPopoverEl.style.position = "fixed";
    deps.contextPopoverEl.style.left = `${Math.round(left)}px`;
    deps.contextPopoverEl.style.right = "auto";
    deps.contextPopoverEl.style.width = `${Math.round(width)}px`;
    deps.contextPopoverEl.style.top = `${Math.round(Math.max(12, top))}px`;
    deps.contextPopoverEl.style.bottom = "auto";
    deps.contextPopoverEl.style.zIndex = "10000";
  }

  function openContextPopover(): void {
    if (!deps.contextPopoverEl || !deps.contextMeterEl) return;
    deps.closeModelMenu();
    // Keep the panel on <body> so composer overflow cannot clip it.
    if (deps.contextPopoverEl.parentElement !== document.body) {
      document.body.appendChild(deps.contextPopoverEl);
    }
    deps.contextPopoverEl.hidden = false;
    deps.contextPopoverEl.classList.add("is-open");
    deps.contextMeterEl.classList.add("is-open");
    deps.contextMeterEl.setAttribute("aria-expanded", "true");
    deps.contextMeterEl.closest(".chat-context-wrap")?.classList.add("is-open");
    deps.composerBoxEl.classList.add("is-context-open");
    deps.composerEl.classList.add("is-context-open");
    // Measure after paint so height is correct for flip-above/below.
    positionContextPopover();
    window.requestAnimationFrame(() => positionContextPopover());
  }

  function toggleContextPopover(): void {
    if (!deps.contextPopoverEl || deps.contextMeterEl?.hidden) return;
    if (deps.contextPopoverEl.hidden) openContextPopover();
    else closeContextPopover();
  }

  function updateContextMeter(): void {
    if (!deps.contextMeterEl || !deps.contextRingFillEl || !deps.contextLabelEl) return;
    const session = deps.activeSession();
    const turns = session.messages.filter((m) => m.role === "user" || m.role === "assistant");
    const hasContent = turns.length > 0 || Boolean(deps.promptEl.value.trim());
    // Always show the meter so a new empty chat can still open the tip panel.
    deps.contextMeterEl.hidden = false;
    const pct = hasContent ? deps.contextUsagePercent(session, deps.promptEl.value) : 0;
    const offset = CONTEXT_RING_LENGTH * (1 - pct / 100);
    deps.contextRingFillEl.style.strokeDasharray = String(CONTEXT_RING_LENGTH);
    deps.contextRingFillEl.style.strokeDashoffset = String(offset);
    deps.contextLabelEl.textContent = `${pct}%`;
    deps.contextMeterEl.title = t("chat.contextMeterTitle");
    deps.contextMeterEl.classList.toggle("is-warn", pct >= 70 && pct < 90);
    deps.contextMeterEl.classList.toggle("is-full", pct >= 90);
    if (deps.contextPopoverTitleEl) {
      deps.contextPopoverTitleEl.textContent = hasContent
        ? t("chat.contextPopoverTitle", { pct: String(pct) })
        : t("chat.contextPopoverTitleEmpty");
    }
    if (deps.contextPopoverBodyEl) {
      deps.contextPopoverBodyEl.textContent = !hasContent
        ? t("chat.contextPopoverBodyEmpty")
        : pct >= 70
          ? t("chat.contextNearFull")
          : t("chat.contextPopoverBody", { pct: String(pct) });
    }
    if (deps.contextCompactEl) {
      const canCompact = turns.length > COMPACT_KEEP_TURNS && !deps.isComposerLocked();
      deps.contextCompactEl.disabled = !canCompact;
      deps.contextCompactEl.hidden = !hasContent;
    }
    if (deps.contextPopoverEl && !deps.contextPopoverEl.hidden) {
      positionContextPopover();
    }
  }

  return {
    closeContextPopover,
    positionContextPopover,
    openContextPopover,
    toggleContextPopover,
    updateContextMeter,
  };
}
