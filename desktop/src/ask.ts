import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { t } from "./i18n";

export type AskRuntime = "claude-code" | "codex" | "hermes" | "openclaw";

type PromptSessionStatus = "succeeded" | "failed" | "cancelled" | "timed_out";

interface PromptSessionReport {
  session_id: string;
  runtime: string;
  cwd: string;
  status: PromptSessionStatus;
  exit_code: number | null;
  summary: string;
  log_excerpt: string;
  duration_ms: number;
}

type PromptSessionEvent =
  | { type: "started"; session_id: string; runtime: string; cwd: string; command: string }
  | { type: "status"; session_id: string; phase: string; message: string }
  | { type: "delta"; session_id: string; text: string }
  | { type: "stdout_line"; session_id: string; line: string }
  | { type: "stderr_line"; session_id: string; line: string }
  | {
      type: "permission_request";
      session_id: string;
      request_id: string;
      tool_name: string;
      detail: string;
      input_json: string;
    }
  | {
      type: "permission_resolved";
      session_id: string;
      request_id: string;
      allowed: boolean;
    }
  | {
      type: "completed";
      session_id: string;
      status: PromptSessionStatus;
      exit_code: number | null;
      summary: string;
    };

const MAX_LOG_CHARS = 120_000;

export type AskPanelHooks = {
  /** Switch to wiring tab */
  goWiring: () => void;
  /** Run diagnose for the selected runtime (card action) */
  diagnoseRuntime: (runtime: AskRuntime) => void;
  /** Open official interactive session */
  openOfficial: (runtime: AskRuntime) => void;
  /** Whether the runtime appears installed from last doctor scan */
  isInstalled: (runtime: AskRuntime) => boolean;
};

let busy = false;
let unlisten: UnlistenFn | null = null;

function els() {
  return {
    panel: document.querySelector<HTMLElement>("#ask-panel"),
    runtime: document.querySelector<HTMLSelectElement>("#ask-runtime"),
    prompt: document.querySelector<HTMLTextAreaElement>("#ask-prompt"),
    elevated: document.querySelector<HTMLInputElement>("#ask-elevated"),
    send: document.querySelector<HTMLButtonElement>("#ask-send"),
    stop: document.querySelector<HTMLButtonElement>("#ask-stop"),
    log: document.querySelector<HTMLPreElement>("#ask-log"),
    meta: document.querySelector<HTMLElement>("#ask-meta"),
    hint: document.querySelector<HTMLElement>("#ask-hint"),
  };
}

function appendLog(line: string, kind: "out" | "err" | "meta" = "out"): void {
  const { log } = els();
  if (!log) {
    return;
  }
  const prefix = kind === "err" ? "⚠ " : kind === "meta" ? "· " : "";
  log.textContent = `${log.textContent ?? ""}${prefix}${line}\n`;
  if ((log.textContent?.length ?? 0) > MAX_LOG_CHARS) {
    log.textContent = log.textContent.slice(-MAX_LOG_CHARS);
  }
  log.scrollTop = log.scrollHeight;
}

function setBusy(next: boolean): void {
  busy = next;
  const { send, stop, prompt, runtime, elevated } = els();
  send && (send.disabled = next);
  stop && (stop.hidden = !next);
  if (prompt) {
    prompt.disabled = next;
  }
  if (runtime) {
    runtime.disabled = next;
  }
  if (elevated) {
    elevated.disabled = next;
  }
}

function setHint(text: string, tone: "ok" | "warn" | "error" | "muted" = "muted"): void {
  const { hint } = els();
  if (!hint) {
    return;
  }
  hint.hidden = !text;
  hint.textContent = text;
  hint.classList.remove("is-ok", "is-warn", "is-error");
  if (tone === "ok") {
    hint.classList.add("is-ok");
  } else if (tone === "warn") {
    hint.classList.add("is-warn");
  } else if (tone === "error") {
    hint.classList.add("is-error");
  }
}

function selectedRuntime(): AskRuntime {
  const { runtime } = els();
  const value = runtime?.value;
  if (value === "codex" || value === "hermes" || value === "openclaw" || value === "claude-code") {
    return value;
  }
  return "claude-code";
}

function elevatedFlags(runtime: AskRuntime): {
  dangerously_skip_permissions: boolean;
  full_auto: boolean;
} {
  const { elevated } = els();
  const on = Boolean(elevated?.checked);
  return {
    dangerously_skip_permissions: (runtime === "claude-code" || runtime === "hermes") && on,
    full_auto: (runtime === "codex" || runtime === "openclaw") && on,
  };
}

async function ensureListener(): Promise<void> {
  if (unlisten) {
    return;
  }
  unlisten = await listen<PromptSessionEvent>("prompt-session-event", (event) => {
    const payload = event.payload;
    switch (payload.type) {
      case "started":
        appendLog(`${payload.runtime} @ ${payload.cwd}`, "meta");
        appendLog(payload.command, "meta");
        break;
      case "stdout_line":
        appendLog(payload.line, "out");
        break;
      case "stderr_line":
        appendLog(payload.line, "err");
        break;
      case "permission_request":
        appendLog(`[permission] ${payload.tool_name}: ${payload.detail}`, "meta");
        appendLog("(use the chat window Allow/Deny buttons)", "meta");
        break;
      case "permission_resolved":
        appendLog(
          payload.allowed ? `[allowed] ${payload.request_id}` : `[denied] ${payload.request_id}`,
          "meta",
        );
        break;
      case "completed":
        appendLog(
          t("ask.completed", {
            status: payload.status,
            code: payload.exit_code == null ? "—" : String(payload.exit_code),
          }),
          "meta",
        );
        break;
    }
  });
}

export function focusAskPanel(runtime?: AskRuntime): void {
  const { panel, runtime: select, prompt } = els();
  if (runtime && select) {
    select.value = runtime;
    updateElevatedLabel();
  }
  panel?.classList.add("is-focused");
  window.setTimeout(() => panel?.classList.remove("is-focused"), 1200);
  panel?.scrollIntoView({ behavior: "smooth", block: "nearest" });
  prompt?.focus();
}

function updateElevatedLabel(): void {
  const label = document.querySelector<HTMLElement>("[data-ask-elevated-label]");
  if (!label) {
    return;
  }
  const runtime = selectedRuntime();
  if (runtime === "codex") {
    label.textContent = t("ask.elevatedCodex");
  } else if (runtime === "hermes") {
    label.textContent = t("ask.elevatedHermes");
  } else if (runtime === "openclaw") {
    label.textContent = t("ask.elevatedOpenclaw");
  } else {
    label.textContent = t("ask.elevatedClaude");
  }
}

export function initAskPanel(hooks: AskPanelHooks): void {
  const { send, stop, runtime, elevated } = els();
  runtime?.addEventListener("change", updateElevatedLabel);
  updateElevatedLabel();

  send?.addEventListener("click", () => {
    void startAsk(hooks);
  });
  stop?.addEventListener("click", () => {
    void cancelAsk();
  });

  document.querySelectorAll<HTMLButtonElement>("[data-ask-action]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const action = btn.dataset.askAction;
      const rt = selectedRuntime();
      if (action === "wiring") {
        hooks.goWiring();
      } else if (action === "diagnose") {
        hooks.diagnoseRuntime(rt);
      } else if (action === "open") {
        hooks.openOfficial(rt);
      }
    });
  });

  elevated?.addEventListener("change", () => {
    if (elevated.checked) {
      const ok = window.confirm(t("ask.elevatedConfirm"));
      if (!ok) {
        elevated.checked = false;
      }
    }
  });
}

async function cancelAsk(): Promise<void> {
  try {
    await invoke<boolean>("cancel_prompt_session_command");
    setHint(t("ask.cancelling"), "warn");
  } catch (error) {
    setHint(t("ask.cancelFailed", { error: String(error) }), "error");
  }
}

async function startAsk(hooks: AskPanelHooks): Promise<void> {
  if (busy) {
    return;
  }
  const { prompt, log, meta } = els();
  const runtime = selectedRuntime();
  const text = prompt?.value.trim() ?? "";
  if (!text) {
    setHint(t("ask.emptyPrompt"), "warn");
    return;
  }

  if (!hooks.isInstalled(runtime)) {
    setHint(t("ask.notInstalled", { runtime }), "warn");
    return;
  }

  const flags = elevatedFlags(runtime);
  if (
    (flags.dangerously_skip_permissions || flags.full_auto) &&
    !window.confirm(t("ask.elevatedConfirm"))
  ) {
    return;
  }

  await ensureListener();
  if (log) {
    log.textContent = "";
  }
  setBusy(true);
  setHint(t("ask.running"), "muted");
  if (meta) {
    meta.textContent = t("ask.runningMeta", { runtime });
  }

  try {
    const report = await invoke<PromptSessionReport>("start_prompt_session_command", {
      runtime,
      prompt: text,
      cwd: null,
      timeoutSec: 600,
      dangerouslySkipPermissions: flags.dangerously_skip_permissions,
      fullAuto: flags.full_auto,
    });
    const tone =
      report.status === "succeeded"
        ? "ok"
        : report.status === "cancelled"
          ? "warn"
          : "error";
    setHint(
      t("ask.done", {
        status: report.status,
        ms: String(report.duration_ms),
      }),
      tone,
    );
    if (meta) {
      meta.textContent = t("ask.doneMeta", {
        cwd: report.cwd,
        ms: String(report.duration_ms),
      });
    }
    if (report.status === "failed" || report.status === "timed_out") {
      setHint(
        `${t("ask.done", { status: report.status, ms: String(report.duration_ms) })} — ${t("ask.failHint")}`,
        "error",
      );
    }
  } catch (error) {
    setHint(t("ask.failed", { error: String(error) }), "error");
    appendLog(String(error), "err");
  } finally {
    setBusy(false);
  }
}
