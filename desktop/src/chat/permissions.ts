import { invoke } from "@tauri-apps/api/core";
import { t } from "../i18n";
import {
  cleanToolLabel,
  formatPermissionDetail,
  looksLikeToolPayloadJson,
} from "./format";
import { assistantMsgWrap, bubblePlainText } from "./copy-ui";
import type { ChatMessage, ChatSession, PendingPermission, PermissionMeta } from "./types";

export type PermissionsDeps = {
  logEl: HTMLElement;
  isViewingRunningSession: () => boolean;
  runTargetSession: () => ChatSession;
  touchSession: (session: ChatSession) => void;
  saveStore: () => void;
  flushSessionListRender: () => void;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  hideDecisionDock: () => void;
  persistMessage: (
    role: "permission",
    content: string,
    opts?: { permission?: PermissionMeta },
  ) => ChatMessage;
  flushPendingTextSync: () => void;
  sealAssistantBubble: () => void;
  settleActivity: () => void;
  dismissLifecycleActivity: () => void;
  finishToolGroup: (collapse?: boolean) => void;
  getActivityEl: () => HTMLElement | null;
  setActivityEl: (el: HTMLElement | null) => void;
  getToolGroupEl: () => HTMLDetailsElement | null;
  setToolGroupEl: (el: HTMLDetailsElement | null) => void;
  getAssistantBubble: () => HTMLElement | null;
  setAssistantBubble: (el: HTMLElement | null) => void;
  getAssistantMessageId: () => string | null;
  setAssistantMessageId: (id: string | null) => void;
  getAssistantRaw: () => string;
  setAssistantRaw: (raw: string) => void;
};

export type PermissionsApi = ReturnType<typeof createPermissionsController>;

export function createPermissionsController(deps: PermissionsDeps) {
  let pendingPermissionBatch: PendingPermission[] = [];
  let permissionPaintTimer = 0;

  function expireLivePermissionCards(): void {
    if (permissionPaintTimer) {
      window.clearTimeout(permissionPaintTimer);
      permissionPaintTimer = 0;
    }
    pendingPermissionBatch = [];
    for (const card of deps.logEl.querySelectorAll<HTMLElement>(".chat-permission.is-pending")) {
      card.classList.remove("is-pending");
      card.classList.add("is-expired");
      card.dataset.resolved = "1";
      const actions = card.querySelector(".chat-permission-actions");
      if (actions) {
        actions.replaceChildren();
        const badge = document.createElement("span");
        badge.className = "chat-permission-result";
        badge.textContent = t("chat.permissionExpired");
        actions.appendChild(badge);
      }
    }
  }

  function pushPermissionCard(payload: {
    session_id: string;
    request_id: string;
    tool_name: string;
    detail: string;
  }): void {
    deps.flushPendingTextSync();
    deps.sealAssistantBubble();
    if (deps.isViewingRunningSession()) {
      deps.settleActivity();
      deps.dismissLifecycleActivity();
      scrubToolFragmentsBeforePermission(payload.detail);
    }
    if (deps.isViewingRunningSession()) {
      deps.setStatus(t("chat.needYourChoice"), "warn");
    } else {
      deps.setStatus(t("chat.waitingPermissionElsewhere"), "warn");
    }

    if (pendingPermissionBatch.some((item) => item.requestId === payload.request_id)) {
      return;
    }

    const persisted = deps.persistMessage("permission", payload.detail.trim() || payload.tool_name, {
      permission: {
        requestId: payload.request_id,
        toolName: payload.tool_name,
        detail: payload.detail.trim() || payload.tool_name,
        backendSessionId: payload.session_id,
        allowed: null,
      },
    });

    pendingPermissionBatch.push({
      sessionId: payload.session_id,
      requestId: payload.request_id,
      toolName: payload.tool_name,
      detail: payload.detail.trim() || payload.tool_name,
      messageId: persisted.id,
    });

    if (deps.isViewingRunningSession()) {
      deps.hideDecisionDock();
      schedulePaintLivePermissionBatch();
    }
    deps.flushSessionListRender();
  }

  function schedulePaintLivePermissionBatch(): void {
    // Brief coalesce so parallel Bash asks arrive as one card, not single→batch flash.
    if (permissionPaintTimer) window.clearTimeout(permissionPaintTimer);
    permissionPaintTimer = window.setTimeout(() => {
      permissionPaintTimer = 0;
      paintLivePermissionBatch();
    }, 50);
  }

  function clearLivePermissionBatchCard(): void {
    deps.logEl.querySelectorAll<HTMLElement>(".chat-permission.is-batch.is-pending").forEach((el) => {
      el.remove();
    });
    for (const item of pendingPermissionBatch) {
      deps.logEl
        .querySelectorAll<HTMLElement>(
          `.chat-permission.is-pending[data-request-id="${CSS.escape(item.requestId)}"]`,
        )
        .forEach((el) => {
          if (!el.classList.contains("is-batch")) el.remove();
        });
    }
  }

  function paintLivePermissionBatch(): void {
    clearLivePermissionBatchCard();
    if (pendingPermissionBatch.length === 0) {
      return;
    }

    // One pending ask: keep the compact single-tool card.
    if (pendingPermissionBatch.length === 1) {
      const only = pendingPermissionBatch[0];
      const message = deps.runTargetSession().messages.find((m) => m.id === only.messageId);
      if (message) {
        const card = renderPermissionCard(message, true);
        deps.logEl.appendChild(card);
        deps.logEl.scrollTop = deps.logEl.scrollHeight;
      }
      return;
    }

    const card = document.createElement("div");
    card.className = "chat-permission is-pending is-batch";
    card.dataset.batch = "1";

    const head = document.createElement("div");
    head.className = "chat-permission-row";
    const title = document.createElement("span");
    title.className = "chat-permission-summary chat-permission-batch-title";
    title.textContent = t("chat.permissionBatchTitle", {
      count: String(pendingPermissionBatch.length),
    });
    const actions = document.createElement("div");
    actions.className = "chat-permission-actions";
    const allowBtn = document.createElement("button");
    allowBtn.type = "button";
    allowBtn.className = "chat-permission-allow";
    allowBtn.textContent = t("chat.permissionAllowAll");
    const denyBtn = document.createElement("button");
    denyBtn.type = "button";
    denyBtn.className = "chat-permission-deny";
    denyBtn.textContent = t("chat.permissionDenyAll");
    const setBusyLocal = (busyLocal: boolean) => {
      allowBtn.disabled = busyLocal;
      denyBtn.disabled = busyLocal;
    };
    const resolveAll = async (allow: boolean) => {
      if (card.dataset.resolved === "1") return;
      setBusyLocal(true);
      const items = [...pendingPermissionBatch];
      try {
        for (const item of items) {
          await invoke<boolean>("resolve_permission_session_command", {
            sessionId: item.sessionId,
            requestId: item.requestId,
            allow,
          });
        }
      } catch (error) {
        setBusyLocal(false);
        const raw = String(error);
        if (/no active ask session/i.test(raw)) {
          deps.setStatus(t("chat.permissionSessionGone"), "warn");
          expireLivePermissionCards();
        } else {
          deps.setStatus(t("chat.permissionFailed", { error: raw }), "error");
        }
      }
    };
    allowBtn.addEventListener("click", () => void resolveAll(true));
    denyBtn.addEventListener("click", () => void resolveAll(false));
    actions.append(allowBtn, denyBtn);
    head.append(title, actions);
    card.appendChild(head);

    const list = document.createElement("ul");
    list.className = "chat-permission-batch-list";
    for (const item of pendingPermissionBatch) {
      const formatted = formatPermissionDetail(item.detail);
      const li = document.createElement("li");
      li.className = "chat-permission-batch-item";
      li.dataset.requestId = item.requestId;
      const tool = document.createElement("span");
      tool.className = "chat-permission-tool";
      tool.textContent = item.toolName;
      const summary = document.createElement("span");
      summary.className = "chat-permission-summary";
      summary.textContent = formatted.summary || item.toolName;
      summary.title = formatted.full || formatted.summary;
      li.append(tool, summary);
      if (formatted.full && formatted.full !== formatted.summary) {
        const more = document.createElement("details");
        more.className = "chat-permission-more";
        more.open = true;
        const moreSummary = document.createElement("summary");
        moreSummary.textContent = t("chat.permissionExpand");
        const detail = document.createElement("pre");
        detail.className = "chat-permission-detail";
        detail.textContent = formatted.full;
        more.append(moreSummary, detail);
        li.appendChild(more);
      }
      list.appendChild(li);
    }
    card.appendChild(list);
    deps.logEl.appendChild(card);
    deps.logEl.scrollTop = deps.logEl.scrollHeight;
  }

  function scrubToolFragmentsBeforePermission(detail: string): void {
    const formatted = formatPermissionDetail(detail.trim());
    const needles = [formatted.full, formatted.summary, detail.trim()]
      .map((s) => s.replace(/\s+/g, " ").trim())
      .filter((s) => s.length > 8);

    // Only strip ephemeral live tool chips — never the resolved permission history
    // group (`chat-turn-tools` / `chat-permission-group`), or a pending ask vanishes
    // when the next permission arrives.
    while (true) {
      const last = deps.logEl.lastElementChild as HTMLElement | null;
      if (!last?.classList.contains("chat-tool-group")) break;
      if (
        last.classList.contains("chat-turn-tools") ||
        last.classList.contains("chat-permission-group") ||
        !last.classList.contains("is-live")
      ) {
        break;
      }
      last.remove();
    }
    deps.setToolGroupEl(null);
    deps.setActivityEl(null);

    // Remove trailing assistant bubbles that are only the tool JSON / command dump.
    while (true) {
      const last = deps.logEl.lastElementChild as HTMLElement | null;
      const bubble =
        last?.classList.contains("chat-msg-assistant")
          ? last.querySelector<HTMLElement>(":scope > .chat-bubble.assistant")
          : last?.classList.contains("assistant")
            ? last
            : null;
      if (!bubble) break;
      const text = bubblePlainText(bubble).replace(/\s+/g, " ").trim();
      if (!text) {
        removeAssistantBubbleElement(bubble);
        continue;
      }
      const isToolDump =
        looksLikeToolPayloadJson(text) ||
        needles.some((needle) => text === needle || text.includes(needle) || needle.includes(text));
      if (!isToolDump) break;
      removeAssistantBubbleElement(bubble);
    }
  }

  function removeAssistantBubbleElement(el: HTMLElement): void {
    const bubble = el.classList.contains("chat-bubble")
      ? el
      : el.querySelector<HTMLElement>(":scope > .chat-bubble.assistant") ?? el;
    const messageId = bubble.dataset.messageId;
    assistantMsgWrap(bubble).remove();
    if (!messageId) return;
    const session = deps.runTargetSession();
    session.messages = session.messages.filter((m) => m.id !== messageId);
    deps.saveStore();
    if (deps.getAssistantMessageId() === messageId) {
      deps.setAssistantBubble(null);
      deps.setAssistantMessageId(null);
      deps.setAssistantRaw("");
    }
  }

  function renderPermissionCard(message: ChatMessage, interactive: boolean): HTMLElement {
    const meta = message.permission;
    const card = document.createElement("div");
    const allowed = meta?.allowed;
    card.className =
      allowed === true
        ? "chat-permission is-allowed"
        : allowed === false
          ? "chat-permission is-denied"
          : interactive
            ? "chat-permission is-pending"
            : "chat-permission is-expired";
    card.dataset.requestId = meta?.requestId ?? "";
    card.dataset.messageId = message.id;
    if (allowed != null) card.dataset.resolved = "1";

    const toolName = meta?.toolName ?? "tool";
    const formatted = formatPermissionDetail(meta?.detail || message.content);

    const row = document.createElement("div");
    row.className = "chat-permission-row";

    const tool = document.createElement("span");
    tool.className = "chat-permission-tool";
    tool.textContent = toolName;

    const summary = document.createElement("span");
    summary.className = "chat-permission-summary";
    summary.textContent = formatted.summary || t("chat.permissionTitle", { tool: toolName });
    summary.title = formatted.full || formatted.summary;

    const actions = document.createElement("div");
    actions.className = "chat-permission-actions";

    if (interactive && allowed == null && meta) {
      const allowBtn = document.createElement("button");
      allowBtn.type = "button";
      allowBtn.className = "chat-permission-allow";
      allowBtn.textContent = t("chat.permissionAllow");
      const denyBtn = document.createElement("button");
      denyBtn.type = "button";
      denyBtn.className = "chat-permission-deny";
      denyBtn.textContent = t("chat.permissionDeny");
      const setLocalBusy = (busyLocal: boolean) => {
        allowBtn.disabled = busyLocal;
        denyBtn.disabled = busyLocal;
      };
      const resolve = async (allow: boolean) => {
        if (card.dataset.resolved === "1") return;
        setLocalBusy(true);
        try {
          await invoke<boolean>("resolve_permission_session_command", {
            sessionId: meta.backendSessionId ?? "",
            requestId: meta.requestId,
            allow,
          });
        } catch (error) {
          setLocalBusy(false);
          const raw = String(error);
          if (/no active ask session/i.test(raw)) {
            deps.setStatus(t("chat.permissionSessionGone"), "warn");
            expireLivePermissionCards();
          } else {
            deps.setStatus(t("chat.permissionFailed", { error: raw }), "error");
          }
        }
      };
      allowBtn.addEventListener("click", () => void resolve(true));
      denyBtn.addEventListener("click", () => void resolve(false));
      actions.append(allowBtn, denyBtn);
    } else {
      const badge = document.createElement("span");
      badge.className = "chat-permission-result";
      badge.textContent =
        allowed === true
          ? t("chat.permissionAllowed")
          : allowed === false
            ? t("chat.permissionDenied")
            : interactive
              ? t("chat.permissionExpired")
              : t("chat.permissionWaiting");
      actions.appendChild(badge);
    }

    row.append(tool, summary, actions);
    card.appendChild(row);

    if (formatted.full) {
      const showOpen =
        allowed == null &&
        (formatted.full !== formatted.summary || /[\n|&;]/.test(formatted.full));
      const more = document.createElement("details");
      more.className = "chat-permission-more";
      more.open = Boolean(showOpen && interactive);
      const moreSummary = document.createElement("summary");
      moreSummary.textContent = t("chat.permissionExpand");
      const detail = document.createElement("pre");
      detail.className = "chat-permission-detail";
      detail.textContent = formatted.full;
      more.append(moreSummary, detail);
      card.appendChild(more);
    }

    return card;
  }

  function permissionGroupSummaryLabel(count: number, toolNames: string[]): string {
    const allBash = toolNames.length > 0 && toolNames.every((name) => /^bash$/i.test(name));
    if (allBash) {
      return t("chat.permissionGroupBash", { count: String(count) });
    }
    return t("chat.permissionGroupTools", { count: String(count) });
  }

  function isCollapsiblePermissionEl(el: HTMLElement): boolean {
    if (!el.classList.contains("chat-permission")) return false;
    // Live asks and unanswered/expired cards must stay visible — folding them into
    // "$ N tools" makes the Allow/Deny UI look like it flashed away.
    if (el.classList.contains("is-pending")) return false;
    if (el.classList.contains("is-expired")) return false;
    if (el.classList.contains("is-batch")) return false;
    if (el.closest(".chat-permission-group, .chat-turn-tools")) return false;
    return el.classList.contains("is-allowed") || el.classList.contains("is-denied");
  }

  function isTurnToolEphemeral(el: HTMLElement): boolean {
    if (el.classList.contains("is-pending")) return false;
    if (el.classList.contains("chat-turn-tools")) return true;
    if (el.classList.contains("chat-permission-group")) return true;
    if (el.classList.contains("chat-tool-group")) return true;
    if (el.classList.contains("chat-permission")) return isCollapsiblePermissionEl(el);
    if (el.classList.contains("chat-activity") && el.dataset.kind === "tool") return true;
    return false;
  }

  function collectToolNamesFromNode(node: HTMLElement): string[] {
    const names: string[] = [];
    node.querySelectorAll<HTMLElement>(".chat-permission-tool").forEach((el) => {
      const name = el.textContent?.trim();
      if (name) names.push(name);
    });
    if (names.length > 0) return names;
    node.querySelectorAll<HTMLElement>(".chat-tool-cmd, .chat-activity-text").forEach((el) => {
      const label = cleanToolLabel(el.textContent ?? "");
      if (label) names.push(label.split(/\s+/)[0] || label);
    });
    return names;
  }

  function flattenTurnToolNode(node: HTMLElement, stack: HTMLElement): void {
    if (node.classList.contains("chat-turn-tools") || node.classList.contains("chat-permission-group")) {
      const inner =
        node.querySelector<HTMLElement>(".chat-permission-stack, .chat-tool-list") ?? null;
      if (inner) {
        while (inner.firstElementChild) {
          stack.appendChild(inner.firstElementChild);
        }
      }
      node.remove();
      return;
    }
    if (node.classList.contains("chat-tool-group")) {
      const list = node.querySelector<HTMLElement>(".chat-tool-list");
      if (list) {
        while (list.firstElementChild) {
          const row = list.firstElementChild as HTMLElement;
          row.classList.remove("is-live");
          row.classList.add("is-done");
          stack.appendChild(row);
        }
      }
      node.remove();
      return;
    }
    stack.appendChild(node);
  }

  function wrapTurnToolsInGroup(nodes: HTMLElement[]): HTMLDetailsElement {
    const toolNames = nodes.flatMap((node) => collectToolNamesFromNode(node));
    const count = Math.max(
      toolNames.length,
      nodes.reduce((sum, node) => {
        if (node.classList.contains("chat-permission")) return sum + 1;
        const nested =
          node.querySelectorAll(".chat-permission, .chat-activity.kind-tool").length ||
          (node.classList.contains("chat-activity") ? 1 : 0);
        return sum + nested;
      }, 0),
    );
    const group = document.createElement("details");
    group.className = "chat-tool-group chat-turn-tools chat-permission-group";
    group.open = false;

    const summary = document.createElement("summary");
    summary.className = "chat-tool-group-summary";
    const icon = document.createElement("span");
    icon.className = "chat-tool-group-icon";
    icon.setAttribute("aria-hidden", "true");
    icon.textContent = "$";
    const label = document.createElement("span");
    label.className = "chat-tool-group-label";
    label.textContent = permissionGroupSummaryLabel(Math.max(count, 1), toolNames);
    const chevron = document.createElement("span");
    chevron.className = "chat-tool-group-chevron";
    chevron.setAttribute("aria-hidden", "true");
    summary.append(icon, label, chevron);

    const stack = document.createElement("div");
    stack.className = "chat-permission-stack";
    const anchor = nodes[0];
    const parent = anchor.parentElement;
    if (parent) parent.insertBefore(group, anchor);
    for (const node of nodes) {
      flattenTurnToolNode(node, stack);
    }
    group.append(summary, stack);
    return group;
  }

  function collapseResolvedPermissionsBeforeAssistant(anchor: HTMLElement): void {
    deps.finishToolGroup(true);
    const block = assistantMsgWrap(anchor);
    const prev = block.previousElementSibling as HTMLElement | null;
    if (prev?.classList.contains("chat-turn-tools")) {
      const existing = prev as HTMLDetailsElement;
      existing.open = false;
      existing.classList.remove("is-live");
      return;
    }
    const nodes: HTMLElement[] = [];
    let sibling = prev;
    while (sibling && isTurnToolEphemeral(sibling)) {
      nodes.unshift(sibling);
      sibling = sibling.previousElementSibling as HTMLElement | null;
    }
    if (nodes.length === 0) return;
    if (nodes.length === 1 && nodes[0].classList.contains("chat-turn-tools")) {
      const only = nodes[0] as HTMLDetailsElement;
      only.open = false;
      only.classList.remove("is-live");
      return;
    }
    // One leftover live chip is still worth collapsing so it doesn't stay blue forever.
    wrapTurnToolsInGroup(nodes);
  }

  function renderPermissionGroup(messages: ChatMessage[], interactive: boolean): HTMLDetailsElement {
    const cards = messages.map((message) => renderPermissionCard(message, interactive));
    return wrapTurnToolsInGroup(cards);
  }

  function markPermissionResolved(requestId: string, allowed: boolean): void {
    const session = deps.runTargetSession();
    const message = session.messages.find(
      (m) => m.role === "permission" && m.permission?.requestId === requestId,
    );
    if (message?.permission) {
      message.permission.allowed = allowed;
      deps.touchSession(session);
      deps.saveStore();
    }

    const wasInBatch = pendingPermissionBatch.some((item) => item.requestId === requestId);
    pendingPermissionBatch = pendingPermissionBatch.filter((item) => item.requestId !== requestId);
    const stillPending = pendingPermissionBatch.length > 0;

    // Always update the card in the open log if present (even if busy just cleared).
    const card = deps.logEl.querySelector<HTMLElement>(
      `.chat-permission.is-pending[data-request-id="${CSS.escape(requestId)}"]:not(.is-batch)`,
    );
    if (card) {
      card.dataset.resolved = "1";
      card.classList.remove("is-pending", "is-expired");
      card.classList.add(allowed ? "is-allowed" : "is-denied");
      const actions = card.querySelector(".chat-permission-actions");
      if (actions) {
        actions.replaceChildren();
        const badge = document.createElement("span");
        badge.className = "chat-permission-result";
        badge.textContent = allowed ? t("chat.permissionAllowed") : t("chat.permissionDenied");
        actions.appendChild(badge);
      }
    }

    const batchCard = deps.logEl.querySelector<HTMLElement>(".chat-permission.is-batch.is-pending");
    if (batchCard && wasInBatch) {
      if (stillPending) {
        // Batch shrank — repaint remaining asks without tearing down resolved cards.
        schedulePaintLivePermissionBatch();
      } else {
        if (permissionPaintTimer) {
          window.clearTimeout(permissionPaintTimer);
          permissionPaintTimer = 0;
        }
        batchCard.dataset.resolved = "1";
        batchCard.classList.remove("is-pending", "is-expired");
        batchCard.classList.add(allowed ? "is-allowed" : "is-denied");
        const actions = batchCard.querySelector(".chat-permission-actions");
        if (actions) {
          actions.replaceChildren();
          const badge = document.createElement("span");
          badge.className = "chat-permission-result";
          badge.textContent = allowed ? t("chat.permissionAllowed") : t("chat.permissionDenied");
          actions.appendChild(badge);
        }
      }
    }

    deps.hideDecisionDock();
    if (stillPending) {
      deps.setStatus(t("chat.needYourChoice"), "warn");
    } else {
      deps.setStatus(allowed ? t("chat.permissionAllowed") : t("chat.permissionDenied"), "ok");
    }
    deps.flushSessionListRender();
  }


  return {
    get pendingPermissionBatch() {
      return pendingPermissionBatch;
    },
    set pendingPermissionBatch(next: PendingPermission[]) {
      pendingPermissionBatch = next;
    },
    get permissionPaintTimer() {
      return permissionPaintTimer;
    },
    set permissionPaintTimer(next: number) {
      permissionPaintTimer = next;
    },
    expireLivePermissionCards,
    pushPermissionCard,
    schedulePaintLivePermissionBatch,
    clearLivePermissionBatchCard,
    paintLivePermissionBatch,
    scrubToolFragmentsBeforePermission,
    removeAssistantBubbleElement,
    renderPermissionCard,
    collapseResolvedPermissionsBeforeAssistant,
    renderPermissionGroup,
    markPermissionResolved,
  };
}
