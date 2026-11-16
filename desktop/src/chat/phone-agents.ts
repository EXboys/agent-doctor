/**
 * The phone's "which agent / which model" picker: the computer names what it
 * can run, the site carries the owner's pick back on the next job, and the
 * chat applies it before the turn starts. Nothing here touches the DOM or IPC,
 * so the rules stay testable.
 */

export type PhoneAgentRow = { id: string; name: string };

export type PhoneModelRow = { id: string; agent: string; label: string; provider: string };

export type PhonePick = { agent: string; model: string; provider: string };

/** Installed agents, deduped, in the order the computer listed them. */
export function agentRows(installed: Array<{ id: string; label: string }>): PhoneAgentRow[] {
  const rows: PhoneAgentRow[] = [];
  const seen = new Set<string>();
  for (const item of installed) {
    const id = item.id?.trim();
    if (!id || seen.has(id)) continue;
    seen.add(id);
    rows.push({ id, name: item.label?.trim() || id });
  }
  return rows;
}

/**
 * Every model the computer can offer, with the provider's active model first.
 * `agent` is a hint only: providers are shared, so a model may fit several
 * agents. `provider` is what the chat pins so the answer never leaves on a
 * different key. Deduped by provider+id, capped so the sheet stays readable.
 */
export function modelRows(
  providers: Array<{ id?: string | null; model?: string | null; models?: readonly string[] }>,
  label: (id: string) => string,
  agent = "",
  cap = 40,
): PhoneModelRow[] {
  const rows: PhoneModelRow[] = [];
  const seen = new Set<string>();
  for (const provider of providers) {
    const providerId = (provider.id ?? "").trim();
    const candidates = [provider.model ?? "", ...(provider.models ?? [])];
    for (const raw of candidates) {
      const id = (raw ?? "").trim();
      if (!id) continue;
      const key = `${providerId}\n${id}`;
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push({ id, agent, label: label(id) || id, provider: providerId });
      if (rows.length >= cap) return rows;
    }
  }
  return rows;
}

/** What the chat currently runs — shown as the sheet's starting point. */
export function pickOf(
  session:
    | { runtime?: string | null; model?: string | null; providerId?: string | null }
    | null
    | undefined,
): PhonePick {
  return {
    agent: session?.runtime?.trim() ?? "",
    model: session?.model?.trim() ?? "",
    provider: session?.providerId?.trim() ?? "",
  };
}

/**
 * Apply the owner's pick to a chat. Returns whether anything changed.
 * A running turn keeps its runtime; the next one follows the pick, so a
 * switch never cuts an answer in half.
 */
export function applyPick(
  session: {
    runtime?: string | null;
    model?: string | null;
    providerId?: string | null;
    runtimeThreadId?: string | null;
    updatedAt?: number;
  },
  pick: PhonePick,
  opts: { running?: boolean } = {},
): boolean {
  const agent = pick.agent.trim();
  const model = pick.model.trim();
  let changed = false;
  if (agent && !opts.running && session.runtime !== agent) {
    session.runtime = agent;
    // A new agent starts a new thread; its old one belongs to another tool.
    session.runtimeThreadId = null;
    changed = true;
  }
  if (model && session.model !== model) {
    session.model = model;
    // The phone sends the provider its sheet came from: pin it, so the model
    // is asked for on the key that actually serves it.
    session.providerId = pick.provider ? pick.provider : null;
    changed = true;
  } else if (!model && pick.provider && session.providerId !== pick.provider) {
    session.providerId = pick.provider;
    changed = true;
  }
  if (changed && typeof session.updatedAt === "number") session.updatedAt = Date.now();
  return changed;
}

/** One line for the phone strip: "Hermes · deepseek-flash". */
export function pickLabel(
  pick: PhonePick,
  nameOf: (agent: string) => string,
  fallback: string,
): string {
  const agent = pick.agent ? nameOf(pick.agent) : fallback;
  return pick.model ? `${agent} · ${pick.model}` : agent;
}
