/** One Feishu conversation stays one desktop chat. Missing id shares one chat. */
export function channelKeyOf(chat: string | undefined | null): string {
  const key = chat?.trim() ?? "";
  return key || "feishu";
}

export function channelSessionTitle(text: string): string {
  const line = text.trim().split("\n")[0] ?? "";
  if (line.length <= 28) return line;
  return `${line.slice(0, 28)}…`;
}

export type ChannelSessionRef = {
  id: string;
  channelKey?: string | null;
};

/** Find the chat for this Feishu conversation, or place a new one at the front. */
export function placeChannelSession<T extends ChannelSessionRef>(
  sessions: T[],
  chat: string | undefined | null,
  create: () => T,
): { sessions: T[]; session: T; created: boolean } {
  const key = channelKeyOf(chat);
  const existing = sessions.find((session) => session.channelKey === key);
  if (existing) return { sessions, session: existing, created: false };
  const session = create();
  session.channelKey = key;
  return { sessions: [session, ...sessions], session, created: true };
}
