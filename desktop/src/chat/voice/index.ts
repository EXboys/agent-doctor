import { getLocale, t } from "../../i18n";
import { parseVoiceError, type VoiceProvider } from "./types";
import { resolveVoiceProvider } from "./providers";

export type VoiceInputDeps = {
  voiceBtnEl: HTMLButtonElement;
  promptEl: HTMLTextAreaElement;
  isComposerLocked: () => boolean;
  /** Hosted voice already owns the microphone. */
  isHostedActive?: () => boolean;
  setStatus: (text: string, tone?: "ok" | "warn" | "error" | "muted") => void;
  autoResizePrompt: () => void;
  /** Optional override for tests / alternate backends. */
  providers?: VoiceProvider[];
};

export type VoiceInputApi = ReturnType<typeof createVoiceInputController>;

function preferredSpeechLanguage(): string {
  return getLocale() === "zh" ? "zh-CN" : "en-US";
}

function appendToPrompt(promptEl: HTMLTextAreaElement, text: string): void {
  const next = text.trim();
  if (!next) return;
  const cur = promptEl.value;
  if (!cur.trim()) {
    promptEl.value = next;
  } else if (/\s$/.test(cur)) {
    promptEl.value = `${cur}${next}`;
  } else {
    promptEl.value = `${cur} ${next}`;
  }
  promptEl.dispatchEvent(new Event("input", { bubbles: true }));
}

function mapErrorMessage(code: string): string {
  switch (code) {
    case "permission_denied":
      return t("chat.voicePermission");
    case "no_speech":
      return t("chat.voiceNoSpeech");
    case "busy":
      return t("chat.voiceBusy");
    case "cancelled":
      return t("chat.voiceCancelled");
    case "unavailable":
      return t("chat.voiceUnavailable");
    default:
      return t("chat.voiceFailed");
  }
}

export function createVoiceInputController(deps: VoiceInputDeps) {
  let provider: VoiceProvider | null = null;
  let listening = false;
  let starting = false;
  /** Text already in the prompt when this dictation started. */
  let baseline = "";

  function setListeningUi(active: boolean): void {
    listening = active;
    deps.voiceBtnEl.classList.toggle("is-listening", active);
    deps.voiceBtnEl.setAttribute("aria-pressed", active ? "true" : "false");
    deps.voiceBtnEl.title = active ? t("chat.voiceStop") : t("chat.voiceStart");
    deps.voiceBtnEl.setAttribute(
      "aria-label",
      active ? t("chat.voiceStop") : t("chat.voiceStart"),
    );
  }

  function syncEnabled(): void {
    const locked = deps.isComposerLocked();
    const available = !!provider && provider.id !== "null";
    deps.voiceBtnEl.hidden = !available;
    const hosted = deps.isHostedActive?.() ?? false;
    deps.voiceBtnEl.disabled = !available || locked || starting || hosted;
  }

  async function ensureProvider(): Promise<VoiceProvider> {
    if (provider) return provider;
    provider = await resolveVoiceProvider(deps.providers);
    syncEnabled();
    return provider;
  }

  function applyPartial(text: string): void {
    const trimmed = text.trim();
    if (!trimmed) return;
    if (!baseline.trim()) {
      deps.promptEl.value = trimmed;
    } else if (/\s$/.test(baseline)) {
      deps.promptEl.value = `${baseline}${trimmed}`;
    } else {
      deps.promptEl.value = `${baseline} ${trimmed}`;
    }
    deps.autoResizePrompt();
  }

  async function startListening(): Promise<void> {
    if (listening || starting || deps.isComposerLocked()) return;
    const active = await ensureProvider();
    if (active.id === "null") {
      deps.setStatus(t("chat.voiceUnavailable"), "warn");
      syncEnabled();
      return;
    }

    starting = true;
    syncEnabled();
    baseline = deps.promptEl.value;
    setListeningUi(true);
    deps.setStatus(t("chat.voiceListening"), "muted");

    try {
      const result = await active.start({
        language: preferredSpeechLanguage(),
        onPartial: (text) => applyPartial(text),
      });
      // Final text replaces the partial preview built on baseline.
      deps.promptEl.value = baseline;
      appendToPrompt(deps.promptEl, result.text);
      deps.autoResizePrompt();
      deps.promptEl.focus();
      if (result.text.trim()) {
        deps.setStatus(t("chat.voiceDone"), "ok");
      } else {
        deps.setStatus(t("chat.voiceNoSpeech"), "warn");
      }
    } catch (error) {
      const parsed = parseVoiceError(error);
      if (parsed.code !== "cancelled") {
        // Restore baseline if we had been showing partials.
        deps.promptEl.value = baseline;
        deps.autoResizePrompt();
        deps.setStatus(mapErrorMessage(parsed.code), "warn");
      } else {
        deps.setStatus(t("chat.voiceCancelled"), "muted");
      }
    } finally {
      starting = false;
      setListeningUi(false);
      syncEnabled();
    }
  }

  async function stopListening(): Promise<void> {
    if (!listening && !starting) return;
    const active = provider ?? (await ensureProvider());
    try {
      await active.cancel();
    } catch {
      /* ignore */
    }
  }

  async function toggle(): Promise<void> {
    if (listening || starting) {
      await stopListening();
      return;
    }
    await startListening();
  }

  function applyI18n(): void {
    if (!listening) {
      deps.voiceBtnEl.title = t("chat.voiceStart");
      deps.voiceBtnEl.setAttribute("aria-label", t("chat.voiceStart"));
    } else {
      deps.voiceBtnEl.title = t("chat.voiceStop");
      deps.voiceBtnEl.setAttribute("aria-label", t("chat.voiceStop"));
    }
  }

  deps.voiceBtnEl.addEventListener("click", () => {
    void toggle();
  });

  void ensureProvider();

  return {
    syncEnabled,
    applyI18n,
    isListening: () => listening,
    toggle,
    stopListening,
  };
}
