export type WikiProgressPhase = "reading" | "writing" | "finishing";

export type WikiProgress = {
  /** 0–100. Stays under 100 until the run itself finishes. */
  percent: number;
  phase: WikiProgressPhase;
  /** Null until at least one page exists to measure against. */
  remainingMs: number | null;
};

/**
 * Page count is the only signal the wiki run exposes.
 * Before the first page, the bar only creeps so it does not look frozen.
 */
export function wikiRunProgress(input: { elapsedMs: number; done: number; total: number }): WikiProgress {
  const elapsed = Math.max(0, input.elapsedMs);
  const total = Math.max(1, input.total);
  const done = Math.max(0, input.done);
  if (done <= 0) {
    return {
      percent: Math.min(12, Math.round((elapsed / 90_000) * 12)),
      phase: "reading",
      remainingMs: null,
    };
  }
  if (done >= total) {
    return { percent: 92, phase: "finishing", remainingMs: null };
  }
  return {
    percent: Math.min(90, Math.max(8, Math.round((done / total) * 100))),
    phase: "writing",
    remainingMs: Math.round((elapsed / done) * (total - done)),
  };
}
