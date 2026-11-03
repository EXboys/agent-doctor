import type { ChannelSeen } from "./channel-ledger";

/**
 * What the desktop should do when the phone asks to stop a sentence.
 *
 * The stop path has broken three ways: the answer went back to whichever chat
 * was on screen instead of the chat that asked, a stop never reached the job
 * it named, and a stop for a job that had already finished buried the real
 * answer under a "stopped." The decision table below is the fix for all
 * three, kept pure so tests can pin it.
 */
export type PhoneStopFacts = {
  /** A stop for this job already went through; the phone repeats while quiet. */
  asked: boolean;
  /** The ledger entry for this job, so a lost reply can still be replayed. */
  seen: ChannelSeen;
  /** The chat that owns the job still runs a turn. */
  running: boolean;
  /** That chat is the one on screen. */
  active: boolean;
};

export type PhoneStopDecision =
  | { action: "ignore" }
  | { action: "cancel" }
  | { action: "replay"; reply: string; ok: boolean }
  | { action: "missed" };

export function decidePhoneStop(facts: PhoneStopFacts): PhoneStopDecision {
  if (facts.asked) return { action: "ignore" };
  if (facts.running || facts.active) return { action: "cancel" };
  if (facts.seen.kind === "done") {
    return { action: "replay", reply: facts.seen.reply, ok: facts.seen.ok };
  }
  // Nothing runs here and nothing is recorded. The job may belong to another
  // computer or still be alive with its reply in flight, so never claim it
  // was stopped and never close it on the site.
  return { action: "missed" };
}
