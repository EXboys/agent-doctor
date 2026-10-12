type Entry = { state: "started" | "done"; at: number; reply?: string; ok?: boolean };

type Store = Pick<Storage, "getItem" | "setItem">;

const KEY = "agent-doctor.channel.jobs.v1";
const KEEP_MS = 3 * 24 * 60 * 60 * 1000;
const MAX_ENTRIES = 300;

/** What to do with a sentence the site sent, possibly again after a reconnect. */
export type ChannelSeen =
  | { kind: "new" }
  | { kind: "here" }
  | { kind: "done"; reply: string; ok: boolean }
  | { kind: "cut" };

export type ChannelLedger = {
  seen(id: string): ChannelSeen;
  take(id: string): void;
  start(id: string): void;
  unstart(id: string): void;
  finish(id: string, reply: string, ok: boolean): void;
};

export function createChannelLedger(store: Store | null, now: () => number = Date.now): ChannelLedger {
  const thisRun = new Set<string>();

  function read(): Record<string, Entry> {
    try {
      const raw = store?.getItem(KEY);
      const parsed = raw ? (JSON.parse(raw) as unknown) : null;
      return parsed && typeof parsed === "object" ? (parsed as Record<string, Entry>) : {};
    } catch {
      return {};
    }
  }

  function write(entries: Record<string, Entry>): void {
    const cutoff = now() - KEEP_MS;
    const kept = Object.entries(entries)
      .filter(([, entry]) => entry && entry.at >= cutoff)
      .sort((a, b) => b[1].at - a[1].at)
      .slice(0, MAX_ENTRIES);
    try {
      store?.setItem(KEY, JSON.stringify(Object.fromEntries(kept)));
    } catch {
      /* storage full: the site still holds the sentence */
    }
  }

  return {
    seen(id) {
      const entry = read()[id];
      if (entry?.state === "done") {
        return { kind: "done", reply: entry.reply ?? "", ok: entry.ok !== false };
      }
      if (thisRun.has(id)) return { kind: "here" };
      if (entry?.state === "started") return { kind: "cut" };
      return { kind: "new" };
    },
    take(id) {
      thisRun.add(id);
    },
    start(id) {
      thisRun.add(id);
      const entries = read();
      entries[id] = { state: "started", at: now() };
      write(entries);
    },
    unstart(id) {
      const entries = read();
      if (entries[id]?.state !== "started") return;
      delete entries[id];
      write(entries);
    },
    finish(id, reply, ok) {
      thisRun.add(id);
      const entries = read();
      entries[id] = { state: "done", at: now(), reply: reply.slice(0, 8000), ok };
      write(entries);
    },
  };
}
