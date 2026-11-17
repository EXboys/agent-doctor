/**
 * The phone's "which agent / which model" picker: the computer names what it
 * can run, the site carries the owner's pick back on the next job, and the
 * chat applies it before the turn starts. Nothing here touches the DOM or IPC,
 * so the rules stay testable.
 */

export type PhoneAgentRow = { id: string; name: string };

export type PhoneModelRow = {
  id: string;
  agent: string;
  label: string;
  provider: string;
  providerName?: string;
  /** The official (quota-based) provider: the phone tags it 内置 vs 自配 key. */
  builtin?: boolean;
  /** A signed-in subscription / native account on this computer. */
  account?: boolean;
};

export type PhonePick = { agent: string; model: string; provider: string };

export type PhoneProviderInput = {
  id?: string | null;
  name?: string | null;
  agent?: string | null;
  model?: string | null;
  models?: readonly string[];
  builtin?: boolean | null;
  account?: boolean | null;
};

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
 * Build the provider sources for the phone picker:
 * - Official account entries ONLY when the official account is signed in.
 * - Native / subscription account entries ONLY when that runtime is signed in
 *   and installed on this computer.
 * - Saved API-key providers configured by the owner.
 */
export function phoneInventoryProviders(opts: {
  installedIds: ReadonlySet<string>;
  personalProviders: ReadonlyArray<{ id: string; name: string; url: string; model: string }>;
  nativeRows: ReadonlyArray<{
    runtime_id: string;
    logged_in: boolean;
    models?: readonly string[];
  }>;
  official: {
    signedIn: boolean;
    giftLeft: number;
    paidOrTrialReady: boolean;
    model?: string | null;
    officialLabel: string;
  };
  officialProviderId: string;
  officialGiftModel: string;
  modelsForUrl: (url: string, currentModel?: string | null) => string[];
  chipForUrl: (url: string, fallbackName: string) => string;
  accountLabel: (runtimeId: string) => string;
}): PhoneProviderInput[] {
  const out: PhoneProviderInput[] = [];

  if (opts.official.signedIn) {
    if (opts.official.giftLeft > 0) {
      out.push({
        id: opts.officialProviderId,
        name: opts.official.officialLabel,
        agent: "doctor",
        models: [opts.officialGiftModel],
        builtin: true,
      });
    }
    if (opts.official.paidOrTrialReady) {
      out.push({
        id: opts.officialProviderId,
        name: opts.official.officialLabel,
        agent: "",
        models: [opts.official.model?.trim() || "deepseek-flash"],
        builtin: true,
      });
    }
  }

  for (const row of opts.nativeRows) {
    const runtimeId = row.runtime_id?.trim();
    if (!runtimeId || !row.logged_in || !opts.installedIds.has(runtimeId)) continue;
    const models = (row.models ?? []).map((m) => m.trim()).filter(Boolean);
    out.push({
      id: `native:${runtimeId}`,
      name: opts.accountLabel(runtimeId),
      agent: runtimeId,
      models: models.length > 0 ? models : ["auto"],
      account: true,
    });
  }

  for (const provider of opts.personalProviders) {
    if (provider.id === opts.officialProviderId) continue;
    out.push({
      id: provider.id,
      name: opts.chipForUrl(provider.url, provider.name),
      agent: "",
      model: provider.model,
      models: opts.modelsForUrl(provider.url, provider.model),
    });
  }

  return out;
}

/**
 * Every model the computer can offer, with the provider's active model first.
 * `agent` scopes account-bound rows to the agent that owns that login; shared
 * API-key providers keep an empty `agent` so every compatible agent sees them.
 * Deduped by agent+provider+id, capped so the sheet stays readable.
 */
export function modelRows(
  providers: Array<PhoneProviderInput>,
  label: (id: string) => string,
  agent = "",
  cap = 40,
): PhoneModelRow[] {
  const rows: PhoneModelRow[] = [];
  const seen = new Set<string>();
  for (const provider of providers) {
    const providerId = (provider.id ?? "").trim();
    const providerName = (provider.name ?? "").trim();
    const rowAgent = (provider.agent ?? agent ?? "").trim();
    const candidates = [provider.model ?? "", ...(provider.models ?? [])];
    for (const raw of candidates) {
      const id = (raw ?? "").trim();
      if (!id) continue;
      const key = `${rowAgent}\n${providerId}\n${id}`;
      if (seen.has(key)) continue;
      seen.add(key);
      const resolvedLabel =
        id === "auto" && provider.account && providerName
          ? providerName
          : label(id) || id;
      rows.push({
        id,
        agent: rowAgent,
        label: resolvedLabel,
        provider: providerId,
        ...(providerName ? { providerName } : {}),
        ...(provider.builtin ? { builtin: true } : {}),
        ...(provider.account ? { account: true } : {}),
      });
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
  const nextProvider = pick.provider ? pick.provider.trim() : null;
  let changed = false;
  if (agent && !opts.running && session.runtime !== agent) {
    session.runtime = agent;
    // A new agent starts a new thread; its old one belongs to another tool.
    session.runtimeThreadId = null;
    changed = true;
  }
  if (model && (session.model !== model || (session.providerId ?? null) !== nextProvider)) {
    if ((session.providerId ?? null) !== nextProvider) {
      session.runtimeThreadId = null;
    }
    session.model = model;
    // The phone sends the provider its sheet came from: pin it, so the model
    // is asked for on the key (or signed-in account) that actually serves it.
    session.providerId = nextProvider;
    changed = true;
  } else if (!model && nextProvider && session.providerId !== nextProvider) {
    session.providerId = nextProvider;
    session.runtimeThreadId = null;
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
