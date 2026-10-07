import type { AskRuntime, WorkspaceDoc } from "./ask-resources";
import type { PersonalProviderListItem } from "./types";
import type { ChatAttachment, SessionStore } from "./chat/types";
import type { PermissionsApi } from "./chat/permissions";
import type { SessionsApi } from "./chat/sessions";
import type { BubblesApi } from "./chat/bubbles";
import type { StreamApi } from "./chat/stream";
import type { SendApi } from "./chat/send";
import type { ActivityApi } from "./chat/activity";
import type { ThinkingApi } from "./chat/thinking";
import type { RunningApi } from "./chat/running";
import type { ModelPickerApi } from "./chat/model-picker";
import type { DecisionApi } from "./chat/decision";
import type { AttachmentsApi } from "./chat/attachments";
import type { VoiceInputApi } from "./chat/voice";
import type { HostedApi } from "./chat/hosted";
import type { ContextMeterApi } from "./chat/context-meter";
import type { ShellUiApi } from "./chat/shell-ui";
import type { BackupUiApi } from "./chat/backup-ui";
import { createEmptySession, loadStore as loadStoreFromDisk } from "./chat/store";

/** Mutable chat shell. Shared across the entry, wiring, and listeners. */
export const chatState = {
  /** Locked by main-page Ask entry (`#runtime=` / ask-window-focus). Not switched in-chat. */
  currentRuntime: "claude-code" as AskRuntime,
  wiredProvider: null as PersonalProviderListItem | null,
  modelMenuOpen: false,
  store: undefined as unknown as SessionStore,
  busy: false,
  busyGen: 0,
  /** Frontend chat session id for the in-flight ask (null when idle). */
  runningChatSessionId: null as string | null,
  /** Backend prompt-session id for the in-flight ask. */
  runningBackendSessionId: null as string | null,
  /** Sessions that finished while the user was looking elsewhere — show 【完成】 until opened. */
  unseenCompletedSessionIds: new Set<string>(),
  permissions: undefined as unknown as PermissionsApi,
  sessions: undefined as unknown as SessionsApi,
  bubbles: undefined as unknown as BubblesApi,
  stream: undefined as unknown as StreamApi,
  send: undefined as unknown as SendApi,
  activity: undefined as unknown as ActivityApi,
  thinking: undefined as unknown as ThinkingApi,
  running: null as RunningApi | null,
  modelPicker: undefined as unknown as ModelPickerApi,
  decision: undefined as unknown as DecisionApi,
  attachments: undefined as unknown as AttachmentsApi,
  voiceInput: undefined as unknown as VoiceInputApi,
  hosted: null as HostedApi | null,
  latestActivityText: "",
  contextMeter: undefined as unknown as ContextMeterApi,
  shellUi: undefined as unknown as ShellUiApi,
  backupUi: undefined as BackupUiApi | undefined,
  assistantBubble: null as HTMLElement | null,
  assistantMessageId: null as string | null,
  assistantRaw: "",
  activityEl: null as HTMLElement | null,
  lifecycleActivityEl: null as HTMLElement | null,
  toolGroupEl: null as HTMLDetailsElement | null,
  pendingText: "",
  pendingAttachments: [] as ChatAttachment[],
  /** True once any assistant text was rendered this turn (avoids result-fallback duplicates). */
  turnHadAssistantText: false,
  workspaceCwd: null as string | null,
  workspaceDoc: null as WorkspaceDoc | null,
  storePersistTimer: 0,
  sessionListRenderTimer: 0,
  /** When true, this Ask turn is a browser MCP pathway verify. */
  verifyMcpTurn: false,
  verifySawBrowserNavigate: false,
  verifyMcpReported: false,
  verifyTurnText: "",
};

export function initChatStore(): void {
  try {
    chatState.store = loadStoreFromDisk(chatState.currentRuntime);
  } catch (error) {
    console.error("Ask: failed to load chat store", error);
    const session = createEmptySession(chatState.currentRuntime);
    chatState.store = { activeId: session.id, sessions: [session] };
  }
}
