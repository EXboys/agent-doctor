/** Pluggable voice-input providers for Ask composer. */

export type VoiceErrorCode =
  | "unavailable"
  | "permission_denied"
  | "no_speech"
  | "busy"
  | "cancelled"
  | "failed";

export type VoiceCapability = {
  available: boolean;
  backend: string;
  reason?: string | null;
};

export type VoiceResult = {
  text: string;
  confidence: number;
  isFinal: boolean;
};

export type VoicePartialHandler = (text: string) => void;

export type VoiceStartOptions = {
  language?: string;
  onPartial?: VoicePartialHandler;
};

export interface VoiceProvider {
  readonly id: string;
  capability(): Promise<VoiceCapability>;
  start(options?: VoiceStartOptions): Promise<VoiceResult>;
  cancel(): Promise<void>;
}

export function parseVoiceError(raw: unknown): { code: VoiceErrorCode; detail: string } {
  const text = raw instanceof Error ? raw.message : String(raw ?? "");
  const match = text.match(/speech\.([a-z_]+):(.*)$/i);
  if (match) {
    const code = match[1].toLowerCase() as VoiceErrorCode;
    const known: VoiceErrorCode[] = [
      "unavailable",
      "permission_denied",
      "no_speech",
      "busy",
      "cancelled",
      "failed",
    ];
    if (known.includes(code)) {
      return { code, detail: match[2] ?? "" };
    }
  }
  return { code: "failed", detail: text };
}
