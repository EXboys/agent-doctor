import { formatPermissionDetail } from "../chat/format";
import type { PromptSessionEvent } from "../chat/types";
import { looksLikeBrowserToolCall } from "../chat/verify";

export type IslandPendingKind = "choice" | "line" | "secret" | "options";

export type IslandPending = {
  kind: IslandPendingKind;
  requestId: string;
  sessionId: string;
  title: string;
  detail: string;
  inputJson?: string;
  /** What the assistant wants to run, e.g. Edit or Bash. */
  tool?: string;
};

export type IslandTrack = {
  active: boolean;
  browser: boolean;
  detail: string;
  /** Latest words from the assistant. Kept after the turn ends. */
  spoken: string;
  /** The last thing the user sent. Kept after the turn ends. */
  sent: string;
  pending: IslandPending | null;
};

export type IslandLabels = {
  idle: string;
  browser: string;
  working: string;
  needsConfirm: string;
  needsReply: string;
  needsAnswer: string;
};

export type IslandSnapshot = {
  active: boolean;
  browser: boolean;
  composing: boolean;
  title: string;
  detail: string;
  /** How many conversations the expanded card should make room for. */
  rows: number;
  pending: IslandPending | null;
};

export type IslandView = {
  shown: boolean;
  expanded: boolean;
  title: string;
  detail: string;
  attention: boolean;
  pending: IslandPending | null;
};

export function emptyIslandTrack(): IslandTrack {
  return { active: false, browser: false, detail: "", spoken: "", sent: "", pending: null };
}

const READ_LIMIT = 4000;

function tidy(text: string): string {
  return text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => line.replace(/[ \t]+/g, " ").trimEnd())
    .join("\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}

function clipStart(text: string, max = READ_LIMIT): string {
  const one = tidy(text);
  if (!one) return "";
  if (one.length <= max) return one;
  return `${one.slice(0, max - 1)}…`;
}

function clipEnd(text: string, max = READ_LIMIT): string {
  const one = tidy(text);
  if (!one) return "";
  if (one.length <= max) return one;
  return `…${one.slice(one.length - (max - 1))}`;
}

/** Last user line, plus the reply that came after it. */
export function latestExchange(messages: { role: string; content: string }[]): {
  sent: string;
  spoken: string;
} {
  let sentIdx = -1;
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i]?.role === "user" && messages[i].content.trim()) {
      sentIdx = i;
      break;
    }
  }
  if (sentIdx < 0) return { sent: "", spoken: "" };
  let spoken = "";
  for (let i = messages.length - 1; i > sentIdx; i -= 1) {
    if (messages[i]?.role === "assistant" && messages[i].content.trim()) {
      spoken = messages[i].content;
      break;
    }
  }
  return { sent: clipStart(messages[sentIdx].content), spoken: clipStart(spoken) };
}

export function rememberIslandSent(track: IslandTrack, text: string): IslandTrack {
  const sent = clipStart(text);
  if (!sent || sent === track.sent) return track;
  return { ...track, sent, spoken: "" };
}

function shorten(message: string): string {
  const one = message.replace(/\s+/g, " ").trim();
  if (!one) return "";
  return one.length > 72 ? `${one.slice(0, 72)}…` : one;
}

function pendingKind(mode: string | undefined): IslandPendingKind {
  if (mode === "line") return "line";
  if (mode === "secret") return "secret";
  if (mode === "options") return "options";
  return "choice";
}

function browserNoise(message: string): boolean {
  return looksLikeBrowserToolCall(message);
}

export function reduceIslandTrack(track: IslandTrack, event: PromptSessionEvent): IslandTrack {
  switch (event.type) {
    case "started":
      return { ...track, active: true, browser: false, detail: "", spoken: "", pending: null };
    case "status": {
      const browser = track.browser || browserNoise(event.message);
      const nextDetail = browserNoise(event.message) ? track.detail : shorten(event.message);
      return {
        ...track,
        active: true,
        browser,
        detail: nextDetail || track.detail,
      };
    }
    case "delta":
    case "stdout_line": {
      const chunk = event.type === "delta" ? event.text : event.line;
      if (browserNoise(chunk)) {
        return { ...track, active: true, browser: true };
      }
      const spoken = clipEnd(`${track.spoken}${chunk}`);
      return {
        ...track,
        active: true,
        spoken: spoken || track.spoken,
      };
    }
    case "permission_request": {
      const summary = formatPermissionDetail(event.detail).summary;
      const browser =
        track.browser || browserNoise(event.tool_name) || browserNoise(event.detail);
      return {
        ...track,
        active: true,
        browser,
        detail: summary || track.detail,
        pending: {
          kind: pendingKind(event.input_mode),
          requestId: event.request_id,
          sessionId: event.session_id,
          title: "",
          detail: summary,
          inputJson: event.input_json,
          tool: event.tool_name,
        },
      };
    }
    case "permission_resolved":
      if (track.pending?.requestId !== event.request_id) return track;
      return { ...track, pending: null };
    case "completed":
      return {
        active: false,
        browser: false,
        detail: "",
        spoken: track.spoken || clipStart(event.summary),
        sent: track.sent,
        pending: null,
      };
    default:
      return track;
  }
}

export function islandTitle(track: IslandTrack, labels: IslandLabels): string {
  if (!track.active) return labels.idle;
  if (track.pending?.kind === "choice") return labels.needsConfirm;
  if (track.pending?.kind === "line" || track.pending?.kind === "secret") return labels.needsReply;
  if (track.pending?.kind === "options") return labels.needsAnswer;
  if (track.browser) return labels.browser;
  return labels.working;
}

const AGENT_NAMES: Record<string, { name: string; badge: string }> = {
  "claude-code": { name: "Claude Code", badge: "Claude" },
  codex: { name: "Codex", badge: "Codex" },
  hermes: { name: "Hermes", badge: "Hermes" },
  openclaw: { name: "OpenClaw", badge: "OpenClaw" },
  "deepseek-harness": { name: "DeepSeek", badge: "DeepSeek" },
};

export type IslandSessionSource = {
  id: string;
  runtime: string;
  title: string;
  updatedAt: number;
  messages: { role: string; content: string }[];
};

export type IslandRow = {
  id: string;
  agent: string;
  badge: string;
  runtime: string;
  preview: string;
  sent: string;
  spoken: string;
  at: number;
  current: boolean;
  needsYou: boolean;
  /** This conversation is the one turn that is running. */
  working: boolean;
};

function agentOf(runtime: string): { name: string; badge: string } {
  return AGENT_NAMES[runtime] ?? { name: runtime || "助手", badge: runtime || "助手" };
}

function clipPreview(line: string): string {
  if (!line) return "";
  return line.length > 42 ? `${line.slice(0, 42)}…` : line;
}

function previewLine(title: string, sent: string): string {
  const line = (title.trim() || sent).split("\n")[0]?.trim() ?? "";
  return clipPreview(line);
}

/** The outcome, not the original request. "收到密钥了（已按你的要求…）" → "收到密钥了". */
function leadSentence(text: string): string {
  const line = text
    .split("\n")
    .map((part) => part.trim())
    .find(Boolean);
  if (!line) return "";
  const cut = line.split(/[。！？!?（(]/)[0]?.trim() ?? "";
  return cut || line;
}

const QUIET_MS = 30 * 60 * 1000;

/** Recent conversations for the expanded island. Live text wins for the open one. */
export function buildIslandRows(
  sessions: IslandSessionSource[],
  activeId: string,
  live: { sent: string; spoken: string } = { sent: "", spoken: "" },
  opts: { now?: number; needsYouId?: string; workingId?: string } = {},
): IslandRow[] {
  const now = opts.now ?? Date.now();
  const needsYouId = opts.needsYouId ?? "";
  const workingId = opts.workingId ?? "";
  return [...sessions]
    .sort((a, b) => b.updatedAt - a.updatedAt)
    .slice(0, 12)
    .map((session) => {
      const exchange = latestExchange(session.messages ?? []);
      const current = session.id === activeId;
      let sent = exchange.sent;
      let spoken = exchange.spoken;
      if (current && live.sent) {
        sent = live.sent;
        spoken = live.sent === exchange.sent ? live.spoken || exchange.spoken : live.spoken;
      } else if (current && live.spoken) {
        spoken = live.spoken;
      }
      const agent = agentOf(session.runtime);
      const needsYou = Boolean(needsYouId) && session.id === needsYouId;
      const working = !needsYou && Boolean(workingId) && session.id === workingId;
      const done = needsYou ? "" : leadSentence(spoken);
      return {
        id: session.id,
        agent: agent.name,
        badge: agent.badge,
        runtime: session.runtime,
        preview: clipPreview(done) || previewLine(session.title, sent),
        sent,
        spoken,
        at: session.updatedAt,
        current,
        needsYou,
        working,
      };
    })
    .filter((row) => {
      if (row.current || row.needsYou || row.working) return true;
      if (!row.sent && !row.spoken) return false;
      return now - row.at < QUIET_MS;
    })
    .sort((a, b) => rowRank(b) - rowRank(a) || b.at - a.at);
}

function rowRank(row: IslandRow): number {
  if (row.needsYou) return 2;
  if (row.working) return 1;
  return 0;
}

function chipTitle(track: IslandTrack, status: string, current: IslandRow | undefined): string {
  const who = current?.badge || current?.agent || "";
  if (track.pending || track.active) return who ? `${who} · ${status}` : status;
  const bit = current?.preview ?? "";
  if (who && bit) {
    const short = bit.length > 10 ? `${bit.slice(0, 10)}…` : bit;
    return `${who} · ${short}`;
  }
  return who || status;
}

function visibleDetail(track: IslandTrack): string {
  if (track.pending?.detail) return track.pending.detail;
  const sent = track.sent.trim();
  const spoken = track.spoken.trim();
  if (sent && spoken && sent !== spoken) return `${sent}\u0001${spoken}`;
  return spoken || sent || track.detail.trim();
}

export function islandSnapshot(
  track: IslandTrack,
  labels: IslandLabels,
  composing: boolean,
  rows: IslandRow[] = [],
): IslandSnapshot {
  const status = islandTitle(track, labels);
  const pending = track.pending ? { ...track.pending, title: status } : null;
  const current = rows.find((row) => row.needsYou) ?? rows.find((row) => row.current);
  const title = chipTitle(track, status, current);
  return {
    active: track.active,
    browser: track.browser,
    composing,
    title,
    detail: rows.length > 0 ? JSON.stringify({ v: 1, rows }) : visibleDetail(track),
    rows: Math.max(1, rows.length),
    pending,
  };
}
