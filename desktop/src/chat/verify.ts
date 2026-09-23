export function looksLikeBrowserToolCall(message: string): boolean {
  const lower = message.toLowerCase();
  return (
    lower.includes("browser_navigate") ||
    lower.includes("browser_snapshot") ||
    lower.includes("browser_get_text") ||
    lower.includes("browser__browser_") ||
    /mcp__browser__/.test(lower) ||
    /browser\.(navigate|snapshot|click)/.test(lower)
  );
}

/** OpenClaw often replies with the page title and never streams the tool name. */
export function looksLikeBrowserMcpVerifyEvidence(message: string): boolean {
  const lower = message.toLowerCase();
  if (
    lower.includes("browser mcp ready") ||
    lower.includes("browser mcp skipped") ||
    lower.includes("browser mcp wire") ||
    lower.includes("watching for browser")
  ) {
    return false;
  }
  const hasUrl = lower.includes("example.com");
  const hasTitle = lower.includes("example domain");
  const claimedUse =
    /已用\s*browser\s*mcp/.test(message) ||
    /\bused\s+browser\s+mcp\b/.test(lower) ||
    /with\s+browser\s+mcp/.test(lower);
  return (hasUrl && hasTitle) || (claimedUse && (hasUrl || hasTitle));
}

export function withTimeoutChat<T>(promise: Promise<T>, ms: number, timeoutError: Error): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = window.setTimeout(() => reject(timeoutError), ms);
    promise.then(
      (value) => {
        window.clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        window.clearTimeout(timer);
        reject(error);
      },
    );
  });
}
