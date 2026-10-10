import type { AskRuntime } from "../ask-resources";
import type { AgentPlan, PlanStep } from "../plan";
import type { TokenUsage } from "../types";

export type PromptSessionStatus = "succeeded" | "failed" | "cancelled" | "timed_out";
export type ChatRole = "user" | "assistant" | "meta" | "permission" | "tool" | "thinking";
export type AttachKind = "file" | "image";
export type ChatTheme = "light" | "dark";
export type CopyIdleKind = "text" | "code";

export interface PromptSessionReport {
  session_id: string;
  runtime: string;
  cwd: string;
  status: PromptSessionStatus;
  exit_code: number | null;
  summary: string;
  log_excerpt?: string;
  duration_ms: number;
  runtime_thread_id?: string | null;
}

export type PromptSessionEvent =
  | {
      type: "started";
      session_id: string;
      runtime: string;
      cwd: string;
      command: string;
      client_run_id?: string;
    }
  | { type: "status"; session_id: string; phase: string; message: string }
  | { type: "delta"; session_id: string; text: string }
  | { type: "thinking"; session_id: string; text: string }
  | { type: "stdout_line"; session_id: string; line: string }
  | { type: "stderr_line"; session_id: string; line: string }
  | {
      type: "permission_request";
      session_id: string;
      request_id: string;
      tool_name: string;
      detail: string;
      input_json: string;
      input_mode?: string;
    }
  | {
      type: "permission_resolved";
      session_id: string;
      request_id: string;
      allowed: boolean;
    }
  | { type: "plan"; session_id: string; items: PlanStep[] }
  | { type: "usage"; session_id: string; usage: TokenUsage; model?: string | null }
  | {
      type: "completed";
      session_id: string;
      status: PromptSessionStatus;
      exit_code: number | null;
      summary: string;
      /** Set when this app stopped the turn. Absent when the runtime stopped it. */
      timeout?: {
        kind: "idle" | "tool" | "absolute" | string;
        quiet_sec: number;
        elapsed_sec: number;
        last_tool: string;
      } | null;
    };

export interface ChatAttachment {
  id: string;
  path: string;
  name: string;
  kind: AttachKind;
}

export interface PermissionMeta {
  requestId: string;
  toolName: string;
  detail: string;
  /** Backend prompt-session id used for resolve_permission_session_command */
  backendSessionId?: string;
  /** null = pending/expired without decision */
  allowed: boolean | null;
  /** choice = allow/deny, options = structured question, line = one reply, secret = hidden reply */
  inputMode?: "choice" | "options" | "line" | "secret";
  /** Original tool input, used to render structured questions. */
  inputJson?: string;
}

export interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  at: number;
  attachments?: ChatAttachment[];
  permission?: PermissionMeta;
}

export interface ChatSession {
  id: string;
  title: string;
  runtime: AskRuntime;
  createdAt: number;
  updatedAt: number;
  messages: ChatMessage[];
  /** Codex thread id / Claude session id for native resume */
  runtimeThreadId?: string | null;
  /** Provider the native thread was made with. Another provider cannot resume it. */
  providerTag?: string | null;
  /** Latest checklist from the agent. Replaced as items move. */
  plan?: AgentPlan;
  /** Set when the last turn stopped before it finished. The next turn hears what it already did. */
  interrupted?: { status: Exclude<PromptSessionStatus, "succeeded">; at: number } | null;
  /** Registered project from the main app (`workspaces.yaml`). Empty = use default when sending. */
  workspaceName?: string | null;
}

export interface SessionStore {
  activeId: string;
  sessions: ChatSession[];
}

export type PendingPermission = {
  sessionId: string;
  requestId: string;
  toolName: string;
  detail: string;
  messageId: string;
};

export const STORAGE_KEY = "agent-doctor.chat.sessions.v2";
export const STORAGE_BACKUP_KEY = "agent-doctor.chat.sessions.v2.backup";
export const LEGACY_STORAGE_KEY = "agent-doctor.chat.sessions.v1";
export const CHAT_STORE_MAX_BYTES = 2_500_000;
export const CHAT_THEME_KEY = "ad.ask.theme";
export const ASK_VERIFY_DRAFT_KEY = "agent-doctor.ask.verifyDraft";

export const MAX_MESSAGES_PER_SESSION = 120;
export const MAX_SESSIONS = 40;
export const MAX_CONTEXT_MESSAGES = 12;
/** Keep this many user/assistant turns after one-click compact. */
export const COMPACT_KEEP_TURNS = 4;
export const MAX_ATTACHMENTS = 8;
export const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "avif"];
export const SHORT_MSG_COPY_CHARS = 140;
export const SHORT_MSG_COPY_LINES = 2;
export const CONTEXT_RING_LENGTH = 2 * Math.PI * 12;
