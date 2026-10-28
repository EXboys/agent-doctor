/** How long an allow may sit before the card says it did not go through. */
export const ALLOW_ACK_MS = 4_000;

export class AllowAckTimeout extends Error {
  constructor() {
    super("allow-ack-timeout");
    this.name = "AllowAckTimeout";
  }
}

export function isAllowAckTimeout(error: unknown): boolean {
  return error instanceof AllowAckTimeout;
}

export function isAskSessionGone(error: unknown): boolean {
  return /no active ask session/i.test(String(error));
}

/** Choice confirms wait for Allow. A question still waits for an answer. */
export function permissionHoldKind(message: string): "allow" | "answer" {
  return /需要你/.test(message) ? "answer" : "allow";
}

const allowStops = new Set<string>();

/** This chat is about to be stopped because an automatic allow did not go through. */
export function requestAllowStop(chatId: string): void {
  if (chatId.trim()) allowStops.add(chatId);
}

export function consumeAllowStop(chatId: string): boolean {
  if (!allowStops.has(chatId)) return false;
  allowStops.delete(chatId);
  return true;
}

const allowInFlight = new Map<string, Promise<unknown>>();

/** One write per request and decision until that write settles. Allow and deny stay separate. */
export function shareAllow(
  requestId: string,
  allow: boolean,
  send: () => Promise<unknown>,
): Promise<unknown> {
  const key = `${requestId}:${allow ? "allow" : "deny"}`;
  const existing = allowInFlight.get(key);
  if (existing) return existing;
  const work = Promise.resolve().then(send);
  const tracked = work.finally(() => {
    if (allowInFlight.get(key) === tracked) allowInFlight.delete(key);
  });
  allowInFlight.set(key, tracked);
  return tracked;
}

/** One allow write. Resolves on ack, rejects on error or when `ms` passes. */
export function ackAllow(send: () => Promise<unknown>, ms = ALLOW_ACK_MS): Promise<void> {
  return waitForAllowAck(send(), ms).then(() => undefined);
}

function waitForAllowAck<T>(work: Promise<T>, ms: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new AllowAckTimeout()), ms);
    work.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

/**
 * Send one choice allow. A finished failure is sent once more.
 * A write that does not return in time is not sent again: the first write may still land.
 */
export async function resolveChoiceAllow(
  send: () => Promise<unknown>,
  ms = ALLOW_ACK_MS,
): Promise<void> {
  try {
    await waitForAllowAck(send(), ms);
  } catch (error) {
    if (isAllowAckTimeout(error) || isAskSessionGone(error)) throw error;
    await waitForAllowAck(send(), ms);
  }
}
