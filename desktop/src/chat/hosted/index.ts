import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { formatPermissionDetail } from "../format";
import { getLocale, t } from "../../i18n";

type HostedState = {
  active: boolean;
  modelBusy: boolean;
  speaking: boolean;
  permissionSpoken: boolean;
  awaitingPermission: boolean;
  held: string | null;
  queuedReply: string | null;
};

type HostedInput =
  | { type: "enter" }
  | { type: "leave" }
  | { type: "heard"; text: string; statusLine: string; zh: boolean }
  | { type: "modelFinished"; text: string; announce: boolean; zh: boolean; failed?: boolean }
  | { type: "speakFinished" }
  | { type: "permissionNeeded"; summary: string; command: string; zh: boolean }
  | { type: "listenFailed" };

type HostedEffect =
  | { type: "startListen" }
  | { type: "stopListen" }
  | { type: "stopSpeak" }
  | { type: "speak"; text: string }
  | { type: "send"; text: string }
  | { type: "decide"; allow: boolean }
  | { type: "leave" };

type SpeechEventDto =
  | { type: "partial"; text: string }
  | { type: "final"; text: string; confidence?: number }
  | { type: "error"; code: string; detail: string }
  | { type: "cancelled" };

export type HostedDeps = {
  mainEl: HTMLElement;
  dialogModeEl: HTMLButtonElement;
  voiceModeEl: HTMLButtonElement;
  islandEl: HTMLButtonElement;
  islandTitleEl: HTMLElement;
  islandDetailEl: HTMLElement;
  promptEl: HTMLTextAreaElement;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  latestActivity: () => string;
  sendAsk: (opts?: { fromVoice?: boolean }) => Promise<void>;
  stopDictation: () => Promise<void>;
  syncDictation: () => void;
  hideChoice: () => void;
};

function emptyState(): HostedState {
  return {
    active: false,
    modelBusy: false,
    speaking: false,
    permissionSpoken: false,
    awaitingPermission: false,
    held: null,
    queuedReply: null,
  };
}

function useChinese(): boolean {
  return getLocale() === "zh";
}

function speechLanguage(): string {
  return useChinese() ? "zh-CN" : "en-US";
}

function voiceDecision(text: string): "allow" | "deny" | null {
  const normalized = text.replace(/[\s，。！？、,.!?]/g, "").toLowerCase();
  if (!normalized) return null;
  if (
    normalized.includes("不允许") ||
    normalized.includes("拒绝") ||
    normalized === "不行" ||
    normalized === "不可以" ||
    normalized === "no" ||
    normalized === "deny"
  ) {
    return "deny";
  }
  if (
    normalized.includes("允许") ||
    normalized.includes("同意") ||
    normalized === "allow" ||
    normalized === "yes"
  ) {
    return "allow";
  }
  return null;
}

function permissionBrief(summary: string, command: string): string {
  const blob = `${summary}\n${command}`.toLowerCase();
  const high =
    /api[_-]?key|secret|password|credential|\btoken\b|sudo|\brm\b|curl|wget|chmod|\.env|settings\.json|printenv/.test(
      blob,
    );
  const medium = /\bnpm\b|\bpnpm\b|\byarn\b|git\s+(commit|push|reset|clean)|\binstall\b/.test(blob);
  if (useChinese()) {
    if (high) {
      return "它想查看设置或密钥。危险程度高，不建议允许。说「允许」或「拒绝」。没听清就问「确认的是什么」。";
    }
    if (medium) {
      return "它想在这台电脑上执行一条命令。危险程度中等，请你看完屏幕上的内容再决定。说「允许」或「拒绝」。没听清就问「确认的是什么」。";
    }
    return "它想做几项只读检查，看看这台电脑的情况。危险程度低，可以允许。说「允许」或「拒绝」。没听清就问「确认的是什么」。";
  }
  if (high) {
    return "It wants to look at settings or secrets. Risk is high. Allowing is not recommended. Say allow or deny.";
  }
  if (medium) {
    return "It wants to run a command on this computer. Risk is medium. Read the screen, then say allow or deny.";
  }
  return "It wants to run a few checks that only look. Risk is low. Allowing is fine. Say allow or deny.";
}

function failureCopy(raw: string): string {
  if (/permission_denied/i.test(raw)) return t("chat.hostedPermission");
  if (/unavailable/i.test(raw)) return t("chat.hostedUnavailable");
  if (/no_speech|no speech|no match/i.test(raw)) return t("chat.hostedNoSpeech");
  if (/busy/i.test(raw)) return t("chat.hostedBusy");
  if (/not found|unknown command|command .* not/i.test(raw)) return t("chat.hostedNeedRestart");
  return t("chat.hostedNoSpeech");
}

export function createHostedController(deps: HostedDeps) {
  let state = emptyState();
  let available = false;
  let partial = "";
  let kept = "";
  let lastUtterance = "";
  let queue: HostedInput[] = [];
  let pumping = false;
  let listenEpoch = 0;
  let listenPromise: Promise<void> | null = null;
  let speakEpoch = 0;
  let sessionUnlisten: UnlistenFn | null = null;
  let failDetail = "";
  let partialTimer: number | null = null;
  let sentUtterance = "";
  let turnWasBusy = false;
  let asking = false;
  let islandError = "";
  let entering = false;
  let enterGuardUntil = 0;
  let confirmLine = "";
  let voicePermission: { sessionId: string; requestId: string } | null = null;

  function render(): void {
    if (state.active) entering = false;
    const on = state.active || entering;
    const busyNow = state.modelBusy || state.speaking;
    if (turnWasBusy && !busyNow) {
      sentUtterance = "";
      asking = false;
    }
    if (state.modelBusy) asking = true;
    turnWasBusy = busyNow;
    deps.mainEl.classList.toggle("is-voice", on);
    const choice = deps.islandEl.parentElement?.querySelector<HTMLElement>("#chat-island-choice");
    const confirming = on && state.awaitingPermission;
    if (choice) choice.hidden = !confirming;
    deps.islandEl.parentElement?.classList.toggle("is-confirming", confirming);
    deps.dialogModeEl.classList.toggle("is-on", !on);
    deps.voiceModeEl.classList.toggle("is-on", on);
    deps.dialogModeEl.setAttribute("aria-pressed", on ? "false" : "true");
    deps.voiceModeEl.setAttribute("aria-pressed", on ? "true" : "false");
    deps.islandEl.setAttribute("aria-pressed", on ? "true" : "false");
    const stage = deps.islandEl.closest<HTMLElement>(".chat-voice-stage");
    if (stage) stage.hidden = !on;

    let title = t("chat.hostedListening");
    let detail = "";
    deps.voiceModeEl.title = t("chat.modeVoiceHint");
    deps.islandEl.classList.remove("is-listening", "is-thinking", "is-speaking");
    if (!on) {
      deps.syncDictation();
      return;
    }
    if (state.awaitingPermission) {
      title = t("chat.hostedConfirm");
      detail = confirmLine;
      deps.islandEl.classList.add(state.speaking ? "is-speaking" : "is-listening");
    } else if (state.speaking) {
      title = t("chat.hostedSpeaking");
      deps.islandEl.classList.add("is-speaking");
    } else if (state.held && state.modelBusy) {
      title = t("chat.hostedHeld");
      deps.islandEl.classList.add("is-thinking");
    } else if (asking || state.modelBusy) {
      title = t("chat.hostedThinking");
      detail = lastUtterance;
      deps.islandEl.classList.add("is-thinking");
    } else {
      detail = t("chat.hostedWillSend");
      deps.islandEl.classList.add("is-listening");
    }
    if (islandError && !state.speaking && !state.modelBusy && !asking) {
      title = t("chat.hostedListenRetry");
      detail = islandError;
      deps.islandEl.classList.add("is-listening");
    }
    if (!asking && !state.modelBusy && !state.speaking && partial.trim()) {
      title = partial.trim();
      detail = t("chat.hostedWillSend");
      deps.islandEl.classList.add("is-listening");
    }
    deps.islandTitleEl.textContent = title;
    deps.islandDetailEl.textContent = detail;
    deps.islandEl.title = state.speaking ? t("chat.islandInterrupt") : t("chat.modeVoiceHint");
    deps.syncDictation();
  }

  async function reduce(input: HostedInput): Promise<void> {
    const step = await invoke<{ state: HostedState; effects: HostedEffect[] }>(
      "voice_hosted_reduce_command",
      { state, input },
    );
    state = step.state ?? state;
    if (voicePermission) state = { ...state, awaitingPermission: true };
    render();
    for (const effect of step.effects ?? []) {
      await runEffect(effect);
    }
  }

  function commitUtterance(text: string): void {
    const heard = text.trim();
    if (!heard) return;
    if (voicePermission) {
      const decision = voiceDecision(heard);
      if (decision) {
        void decidePermission(decision === "allow");
        return;
      }
      if (!state.speaking) void speakPermission(confirmLine || permissionBrief("", ""));
      return;
    }
    if (state.speaking) return;
    if (heard === sentUtterance) return;
    sentUtterance = heard;
    if (partialTimer != null) {
      window.clearTimeout(partialTimer);
      partialTimer = null;
    }
    partial = "";
    kept = "";
    lastUtterance = heard;
    islandError = "";
    deps.setStatus("", "muted");
    render();
    enqueue({
      type: "heard",
      text: heard,
      statusLine: deps.latestActivity(),
      zh: useChinese(),
    });
  }

  const HOLD_MS = 2500;

  function collapseStutter(text: string): string {
    const s = text.trim();
    if (s.length < 12) return s;
    for (let len = Math.floor(s.length / 2); len >= 6; len -= 1) {
      const head = s.slice(0, len);
      if (!s.startsWith(head + head)) continue;
      let i = 0;
      while (i + head.length <= s.length && s.startsWith(head, i)) i += head.length;
      const rest = s.slice(i);
      return `${head}${rest}`;
    }
    return s;
  }

  function joinHeard(earlier: string, latest: string): string {
    const a = earlier.trim();
    const b = latest.trim();
    if (!a) return b;
    if (!b || a === b || a.endsWith(b)) return a;
    if (b.startsWith(a)) return b;
    const gap = /[A-Za-z0-9]$/.test(a) && /^[A-Za-z0-9]/.test(b) ? " " : "";
    return `${a}${gap}${b}`;
  }

  function rememberSpeech(text: string, isFinal: boolean): void {
    const next = collapseStutter(text);
    if (!next) return;
    const before = partial;
    if (isFinal) {
      kept = joinHeard(kept, next);
      partial = kept;
    } else {
      partial = joinHeard(kept, next);
    }
    islandError = "";
    render();
    if (partial !== before) armPartialTimer(partial);
    else if (partialTimer == null) armPartialTimer(partial);
  }

  function armPartialTimer(text: string): void {
    if (partialTimer != null) window.clearTimeout(partialTimer);
    partialTimer = window.setTimeout(() => {
      partialTimer = null;
      if (partial.trim() === text.trim()) commitUtterance(text);
    }, HOLD_MS);
  }

  function enqueue(input: HostedInput): void {
    queue.push(input);
    void pump();
  }

  async function pump(): Promise<void> {
    if (pumping) return;
    pumping = true;
    try {
      while (queue.length > 0) {
        const next = queue.shift();
        if (!next) break;
        await reduce(next);
      }
    } catch (error) {
      console.error("voice chat failed", error);
      asking = false;
      islandError = failureCopy(String(error));
      if (!state.active) entering = false;
      render();
      if (state.active && !state.modelBusy && !state.speaking) {
        enqueue({ type: "listenFailed" });
      }
    } finally {
      pumping = false;
      if (queue.length > 0) void pump();
    }
  }

  async function startListen(): Promise<void> {
    if (listenPromise) return;
    if ((state.modelBusy || state.speaking) && !state.awaitingPermission) return;
    const epoch = ++listenEpoch;
    partial = "";
    kept = "";
    sessionUnlisten = await listen<SpeechEventDto>("speech-event", (event) => {
      if (epoch !== listenEpoch) return;
      const payload = event.payload;
      if (payload?.type === "partial" && typeof payload.text === "string") {
        rememberSpeech(payload.text, false);
        return;
      }
      if (payload?.type === "final" && typeof payload.text === "string" && payload.text.trim()) {
        rememberSpeech(payload.text, true);
      }
    });
    const detach = sessionUnlisten;
    listenPromise = (async () => {
      let lastError: unknown = null;
      for (let attempt = 0; attempt < 5; attempt += 1) {
        if (epoch !== listenEpoch) return;
        try {
          await invoke("voice_listen_start_command", { language: speechLanguage() });
          if (epoch === listenEpoch && state.active) {
            islandError = "";
            render();
          }
          return;
        } catch (error) {
          lastError = error;
          if (epoch !== listenEpoch || !/busy/i.test(String(error))) throw error;
          await new Promise((resolve) => window.setTimeout(resolve, 250));
        }
      }
      throw lastError;
    })()
      .catch((error: unknown) => {
        console.error("voice listen failed", error);
        if (epoch !== listenEpoch || !state.active) return;
        islandError = failureCopy(String(error));
        render();
        window.setTimeout(() => enqueue({ type: "listenFailed" }), 0);
      })
      .finally(() => {
        if (detach) {
          try {
            detach();
          } catch {
            /* ignore */
          }
        }
        if (epoch === listenEpoch) {
          sessionUnlisten = null;
          listenPromise = null;
        }
      });
  }

  async function stopListen(): Promise<void> {
    listenEpoch += 1;
    partial = "";
    kept = "";
    const pending = listenPromise;
    listenPromise = null;
    try {
      await invoke("voice_listen_stop_command");
    } catch {
      /* ignore */
    }
    void pending;
  }

  async function speak(text: string): Promise<void> {
    const epoch = ++speakEpoch;
    try {
      await invoke("voice_speak_command", { text, language: speechLanguage() });
    } catch (error) {
      const raw = String(error);
      if (epoch === speakEpoch && !/cancelled/i.test(raw) && state.active && !state.awaitingPermission) {
        deps.setStatus(t("chat.hostedFailed"), "warn");
      }
    }
    if (epoch === speakEpoch && state.active) enqueue({ type: "speakFinished" });
  }

  async function decidePermission(allow: boolean): Promise<void> {
    const pending = voicePermission;
    voicePermission = null;
    confirmLine = "";
    speakEpoch += 1;
    state = { ...state, awaitingPermission: false, speaking: false };
    asking = false;
    render();
    try {
      await invoke("voice_speak_stop_command");
    } catch {
      /* ignore */
    }
    await stopListen();
    if (!pending) return;
    try {
      await invoke("resolve_permission_session_command", {
        sessionId: pending.sessionId,
        requestId: pending.requestId,
        allow,
      });
    } catch (error) {
      confirmLine = t("chat.hostedConfirmFailed");
      voicePermission = pending;
      state = { ...state, awaitingPermission: true };
      render();
      console.error("voice permission failed", error);
    }
  }

  async function speakPermission(script: string): Promise<void> {
    confirmLine = script;
    state = { ...state, awaitingPermission: true, speaking: true };
    asking = false;
    render();
    const epoch = ++speakEpoch;
    await stopListen();
    try {
      await invoke("voice_speak_stop_command");
    } catch {
      /* ignore */
    }
    try {
      await invoke("voice_speak_command", { text: script, language: speechLanguage() });
    } catch (error) {
      console.error("voice confirm failed", error);
    }
    if (epoch !== speakEpoch || !state.active || !voicePermission) return;
    state = { ...state, speaking: false, awaitingPermission: true };
    render();
    await startListen();
  }

  function sendText(text: string): void {
    deps.promptEl.value = text;
    void deps.sendAsk({ fromVoice: true });
  }

  async function runEffect(effect: HostedEffect): Promise<void> {
    switch (effect.type) {
      case "startListen":
        await startListen();
        break;
      case "stopListen":
        await stopListen();
        break;
      case "stopSpeak":
        speakEpoch += 1;
        try {
          await invoke("voice_speak_stop_command");
        } catch {
          /* ignore */
        }
        break;
      case "speak":
        if (voicePermission) break;
        confirmLine = effect.text;
        await speak(effect.text);
        break;
      case "decide":
        await decidePermission(effect.allow);
        break;
      case "send":
        sendText(effect.text);
        break;
      case "leave":
        if (failDetail) {
          deps.setStatus(failureCopy(failDetail), "warn");
          failDetail = "";
        }
        partial = "";
        kept = "";
        render();
        break;
      default:
        break;
    }
  }

  async function enterVoice(): Promise<void> {
    if (state.active || entering) return;
    if (!available) {
      deps.setStatus(t("chat.hostedUnavailable"), "warn");
      return;
    }
    entering = true;
    enterGuardUntil = Date.now() + 800;
    deps.hideChoice();
    render();
    await new Promise((resolve) => {
      window.requestAnimationFrame(() => window.requestAnimationFrame(() => resolve(undefined)));
    });
    try {
      await deps.stopDictation();
    } catch {
      /* ignore */
    }
    deps.setStatus("", "muted");
    islandError = "";
    asking = false;
    enqueue({ type: "enter" });
  }

  async function leaveVoice(): Promise<void> {
    if (Date.now() < enterGuardUntil) return;
    entering = false;
    if (!state.active && !state.speaking) return;
    enqueue({ type: "leave" });
  }

  async function interruptSpeech(): Promise<void> {
    if (!state.speaking) return;
    try {
      await invoke("voice_speak_stop_command");
    } catch {
      /* ignore */
    }
  }

  async function refreshCapability(): Promise<void> {
    try {
      const cap = await invoke<{ available?: boolean }>("speech_capability_command");
      available = !!cap?.available;
    } catch {
      available = false;
    }
    render();
  }

  let lastVoicePress = 0;
  const pressVoice = (event: Event) => {
    event.stopPropagation();
    const now = Date.now();
    if (now - lastVoicePress < 400) return;
    lastVoicePress = now;
    enterGuardUntil = now + 800;
    void enterVoice();
  };
  const pressDialog = (event: Event) => {
    if (Date.now() < enterGuardUntil) {
      event.preventDefault();
      event.stopPropagation();
      return;
    }
    void leaveVoice();
  };
  deps.voiceModeEl.addEventListener("pointerdown", pressVoice);
  deps.voiceModeEl.addEventListener("click", pressVoice);
  deps.dialogModeEl.addEventListener("pointerdown", pressDialog);
  deps.dialogModeEl.addEventListener("click", pressDialog);
  deps.islandEl.addEventListener("click", () => {
    void interruptSpeech();
  });
  deps.islandEl.parentElement
    ?.querySelector("#chat-island-allow")
    ?.addEventListener("click", () => {
      void decidePermission(true);
    });
  deps.islandEl.parentElement
    ?.querySelector("#chat-island-deny")
    ?.addEventListener("click", () => {
      void decidePermission(false);
    });
  void refreshCapability();
  render();

  return {
    applyI18n: render,
    isActive: () => state.active,
    noteTurnCompleted(text: string, status: string) {
      if (!state.active) return;
      if (status === "cancelled" || status === "succeeded" || !state.modelBusy) {
        asking = false;
        if (status === "cancelled" || status === "succeeded") {
          voicePermission = null;
          confirmLine = "";
          speakEpoch += 1;
        }
        state = {
          ...state,
          modelBusy: status === "succeeded" ? state.modelBusy : false,
          speaking: false,
          awaitingPermission: false,
        };
        render();
      }
      if (!state.modelBusy && status !== "cancelled") return;
      const failed = status !== "succeeded" && status !== "cancelled";
      enqueue({
        type: "modelFinished",
        text,
        announce: failed || (status === "succeeded" && text.trim().length > 0),
        failed,
        zh: useChinese(),
      });
    },
    notePermission(payload: {
      session_id: string;
      request_id: string;
      tool_name: string;
      detail: string;
    }) {
      if (!state.active) return;
      if (voicePermission?.requestId === payload.request_id) return;
      voicePermission = {
        sessionId: payload.session_id,
        requestId: payload.request_id,
      };
      const formatted = formatPermissionDetail(payload.detail || payload.tool_name);
      const script = permissionBrief(
        formatted.summary || payload.tool_name,
        formatted.full || payload.detail || "",
      );
      void speakPermission(script);
    },
  };
}

export type HostedApi = ReturnType<typeof createHostedController>;
