import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  VoiceCapability,
  VoiceProvider,
  VoiceResult,
  VoiceStartOptions,
} from "./types";

type SpeechCapabilityDto = {
  available: boolean;
  backend: string;
  reason?: string | null;
};

type SpeechResultDto = {
  text: string;
  confidence: number;
  isFinal: boolean;
};

type SpeechEventDto =
  | { type: "partial"; text: string }
  | { type: "final"; text: string; confidence: number }
  | { type: "error"; code: string; detail: string }
  | { type: "cancelled" };

/** Native OS speech via Tauri (`SFSpeechRecognizer` / Windows Media Speech). */
export function createNativeVoiceProvider(): VoiceProvider {
  return {
    id: "native",
    async capability() {
      const cap = await invoke<SpeechCapabilityDto>("speech_capability_command");
      return {
        available: !!cap.available,
        backend: cap.backend,
        reason: cap.reason,
      };
    },
    async start(options: VoiceStartOptions = {}) {
      let unlisten: UnlistenFn | undefined;
      try {
        if (options.onPartial) {
          unlisten = await listen<SpeechEventDto>("speech-event", (event) => {
            const payload = event.payload;
            if (payload?.type === "partial" && typeof payload.text === "string") {
              options.onPartial?.(payload.text);
            }
          });
        }
        const result = await invoke<SpeechResultDto>("speech_dictate_command", {
          language: options.language ?? null,
        });
        return {
          text: result.text ?? "",
          confidence: result.confidence ?? 0,
          isFinal: result.isFinal ?? true,
        };
      } finally {
        if (unlisten) {
          try {
            unlisten();
          } catch {
            /* ignore */
          }
        }
      }
    },
    async cancel() {
      await invoke("speech_cancel_dictation_command");
    },
  };
}

/** Hidden / no-op provider when speech is off or unsupported. */
export function createNullVoiceProvider(reason = "disabled"): VoiceProvider {
  return {
    id: "null",
    async capability(): Promise<VoiceCapability> {
      return { available: false, backend: "null", reason };
    },
    async start(): Promise<VoiceResult> {
      throw new Error("speech.unavailable:voice input disabled");
    },
    async cancel() {},
  };
}

/**
 * Registry: first provider whose capability().available is true wins.
 * Add more providers here (e.g. web-speech) without touching the controller.
 */
export async function resolveVoiceProvider(
  candidates: VoiceProvider[] = [createNativeVoiceProvider()],
): Promise<VoiceProvider> {
  for (const provider of candidates) {
    try {
      const cap = await provider.capability();
      if (cap.available) return provider;
    } catch {
      /* try next */
    }
  }
  return createNullVoiceProvider("none_available");
}
