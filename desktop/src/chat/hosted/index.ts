import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getLocale, t } from "../../i18n";

type HostedState = {
  active: boolean;
  modelBusy: boolean;
  speaking: boolean;
  permissionSpoken: boolean;
  held: string | null;
  queuedReply: string | null;
};

type HostedInput =
  | { type: "enter" }
  | { type: "leave" }
  | { type: "heard"; text: string; statusLine: string; zh: boolean }
  | { type: "modelFinished"; text: string; announce: boolean; zh: boolean }
  | { type: "speakFinished" }
  | { type: "permissionNeeded"; zh: boolean }
  | { type: "listenFailed" };

type HostedEffect =
  | { type: "startListen" }
  | { type: "stopListen" }
  | { type: "stopSpeak" }
  | { type: "speak"; text: string }
  | { type: "send"; text: string }
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
};

function emptyState(): HostedState {
  return {
    active: false,
    modelBusy: false,
    speaking: false,
    permissionSpoken: false,
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

function failureCopy(raw: string): string {
  if (/permission_denied/i.test(raw)) return t("chat.hostedPermission");
  if (/unavailable/i.test(raw)) return t("chat.hostedUnavailable");
  if (/not found|unknown command|command .* not/i.test(raw)) return t("chat.hostedNeedRestart");
  return t("chat.hostedFailed");
}

export function createHostedController(deps: HostedDeps) {
  let state = emptyState();
  let available = false;
  let partial = "";
  let lastUtterance = "";
  let queue: HostedInput[] = [];
  let pumping = false;
  let listenEpoch = 0;
  let listenPromise: Promise<void> | null = null;
  let sessionUnlisten: UnlistenFn | null = null;
  let turnWatch = 0;
  let failDetail = "";
  let partialTimer: number | null = null;
  let sentUtterance = "";
  let turnWasBusy = false;
  let asking = false;
  let islandError = "";

  function render(): void {
    const on = state.active;
    const busyNow = state.modelBusy || state.speaking;
    if (turnWasBusy && !busyNow) {
      sentUtterance = "";
      asking = false;
    }
    if (state.modelBusy) asking = true;
    turnWasBusy = busyNow;
    deps.mainEl.classList.toggle("is-voice", on);
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
    if (state.speaking) {
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
      deps.islandEl.classList.add("is-listening");
    }
    if (islandError && !state.speaking) {
      detail = islandError;
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
    render();
    for (const effect of step.effects ?? []) {
      await runEffect(effect);
    }
  }

  function commitUtterance(text: string): void {
    const heard = text.trim();
    if (!heard || state.speaking) return;
    if (heard === sentUtterance) return;
    sentUtterance = heard;
    if (partialTimer != null) {
      window.clearTimeout(partialTimer);
      partialTimer = null;
    }
    partial = "";
    lastUtterance = heard;
    asking = true;
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

  function armPartialTimer(text: string): void {
    if (partialTimer != null) window.clearTimeout(partialTimer);
    partialTimer = window.setTimeout(() => {
      partialTimer = null;
      if (partial.trim() === text.trim()) commitUtterance(text);
    }, 1000);
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
      deps.setStatus(islandError, "warn");
      render();
    } finally {
      pumping = false;
      if (queue.length > 0) void pump();
    }
  }

  async function startListen(): Promise<void> {
    if (listenPromise) return;
    const epoch = ++listenEpoch;
    partial = "";
    sessionUnlisten = await listen<SpeechEventDto>("speech-event", (event) => {
      if (epoch !== listenEpoch) return;
      const payload = event.payload;
      if (payload?.type === "partial" && typeof payload.text === "string") {
        partial = payload.text;
        render();
        if (payload.text.trim()) armPartialTimer(payload.text);
        return;
      }
      if (payload?.type === "final" && typeof payload.text === "string" && payload.text.trim()) {
        commitUtterance(payload.text);
      }
    });
    listenPromise = (async () => {
      let lastError: unknown = null;
      for (let attempt = 0; attempt < 5; attempt += 1) {
        if (epoch !== listenEpoch) return;
        try {
          await invoke("voice_listen_start_command", { language: speechLanguage() });
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
        failDetail = String(error);
        window.setTimeout(() => enqueue({ type: "listenFailed" }), 0);
      })
      .finally(() => {
        if (sessionUnlisten) {
          try {
            sessionUnlisten();
          } catch {
            /* ignore */
          }
          sessionUnlisten = null;
        }
        if (epoch === listenEpoch) listenPromise = null;
      });
  }

  async function stopListen(): Promise<void> {
    listenEpoch += 1;
    partial = "";
    try {
      await invoke("voice_listen_stop_command");
    } catch {
      /* ignore */
    }
    if (listenPromise) {
      try {
        await listenPromise;
      } catch {
        /* ignore */
      }
      listenPromise = null;
    }
  }

  async function speak(text: string): Promise<void> {
    try {
      await invoke("voice_speak_command", { text, language: speechLanguage() });
    } catch (error) {
      const raw = String(error);
      if (!/cancelled/i.test(raw) && state.active) {
        deps.setStatus(t("chat.hostedFailed"), "warn");
      }
    }
    if (state.active) enqueue({ type: "speakFinished" });
  }

  function sendText(text: string): void {
    deps.promptEl.value = text;
    const watch = ++turnWatch;
    void deps.sendAsk({ fromVoice: true }).finally(() => {
      window.setTimeout(() => {
        if (watch === turnWatch && state.active && state.modelBusy) {
          enqueue({
            type: "modelFinished",
            text: "",
            announce: false,
            zh: useChinese(),
          });
        }
      }, 500);
    });
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
        try {
          await invoke("voice_speak_stop_command");
        } catch {
          /* ignore */
        }
        break;
      case "speak":
        await speak(effect.text);
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
        render();
        break;
      default:
        break;
    }
  }

  async function enterVoice(): Promise<void> {
    if (state.active) return;
    if (!available) {
      deps.setStatus(t("chat.hostedUnavailable"), "warn");
      return;
    }
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

  deps.dialogModeEl.addEventListener("click", () => {
    void leaveVoice();
  });
  deps.voiceModeEl.addEventListener("click", () => {
    void enterVoice();
  });
  deps.islandEl.addEventListener("click", () => {
    void interruptSpeech();
  });
  void refreshCapability();
  render();

  return {
    applyI18n: render,
    isActive: () => state.active,
    noteTurnCompleted(text: string, status: string) {
      turnWatch += 1;
      if (!state.active || !state.modelBusy) return;
      enqueue({
        type: "modelFinished",
        text,
        announce: status !== "cancelled" && text.trim().length > 0,
        zh: useChinese(),
      });
    },
    notePermission() {
      if (!state.active) return;
      enqueue({ type: "permissionNeeded", zh: useChinese() });
    },
  };
}

export type HostedApi = ReturnType<typeof createHostedController>;
