//! Long-frame probe for the chat window: a main-thread gap over 50ms while
//! streaming or scrolling is what a person reads as 「卡」, and this counts
//! them instead of trusting feel — the ZCode way (render work is judged by
//! counters, with a before/after table per fix).
//!
//! One rAF loop compares frame timestamps; only a gap over the threshold
//! touches the DOM for attribution (streaming? how many mounted rows), so
//! the probe itself costs nothing per frame. Samples land in a small ring
//! exposed as `window.__AD_PERF`, and a minute with long frames is echoed
//! into the runtime debug log once.

const LONG_TASK_GAP_MS = 50;
const RING_MAX = 400;
const REPORT_WINDOW_MS = 60_000;

export type LongTaskSample = {
  /** Wall clock, for minute bucketing in the report. */
  at: number;
  /** rAF gap that triggered the sample. */
  gapMs: number;
  /** A reply was streaming when the gap ended. */
  streaming: boolean;
  /** Mounted rows in the log at that moment. */
  rows: number;
};

declare global {
  interface Window {
    __AD_PERF?: {
      samples: () => LongTaskSample[];
      report: () => string;
      reset: () => void;
    };
  }
}

/** One line per report window, or "" when the minute was clean. Pure. */
export function summarizeLongTasks(
  samples: LongTaskSample[],
  nowMs: number,
  windowMs = REPORT_WINDOW_MS,
): string {
  const inWindow = samples.filter((sample) => nowMs - sample.at <= windowMs);
  if (inWindow.length === 0) return "";
  const worst = inWindow.reduce((a, b) => (b.gapMs > a.gapMs ? b : a));
  const streaming = inWindow.filter((sample) => sample.streaming).length;
  const parts = [
    `${inWindow.length} long frame(s) >${LONG_TASK_GAP_MS}ms in ${Math.round(windowMs / 1000)}s`,
    `worst ${Math.round(worst.gapMs)}ms`,
  ];
  if (streaming > 0) parts.push(`${streaming} while streaming`);
  parts.push(`rows=${worst.rows}`);
  return parts.join("; ");
}

export function startPerformanceProbe(input: {
  getRows: () => number;
  onReport?: (line: string) => void;
}): void {
  const samples: LongTaskSample[] = [];
  let last = performance.now();
  let raf = 0;
  let lastReportAt = 0;

  const isStreaming = (): boolean =>
    document.querySelector(".chat-log .chat-bubble.is-streaming") !== null;

  const tick = (now: number): void => {
    raf = 0;
    const gap = now - last;
    last = now;
    if (gap >= LONG_TASK_GAP_MS) {
      samples.push({
        at: Date.now(),
        gapMs: Math.round(gap),
        streaming: isStreaming(),
        rows: input.getRows(),
      });
      if (samples.length > RING_MAX) samples.splice(0, samples.length - RING_MAX);
    }
    if (Date.now() - lastReportAt >= REPORT_WINDOW_MS) {
      const line = summarizeLongTasks(samples, Date.now());
      if (line) {
        lastReportAt = Date.now();
        input.onReport?.(line);
      }
    }
    schedule();
  };

  const schedule = (): void => {
    if (!raf && document.visibilityState === "visible") raf = requestAnimationFrame(tick);
  };

  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") {
      // Coming back would read as one huge gap; restart the clock instead.
      last = performance.now();
      schedule();
    } else if (raf) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
  });

  window.__AD_PERF = {
    samples: () => [...samples],
    report: (): string => summarizeLongTasks(samples, Date.now(), Number.MAX_SAFE_INTEGER),
    reset: (): void => {
      samples.length = 0;
      lastReportAt = 0;
    },
  };

  schedule();
}
