/** Official brand marks. Filename (without .svg) is the runtime id. */
const iconModules = import.meta.glob("./assets/agent-icons/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

function iconSvg(runtimeId: string): string | undefined {
  const raw = iconModules[`./assets/agent-icons/${runtimeId}.svg`];
  if (!raw) return undefined;
  // Drop <title> so hovering a tab does not raise a native tooltip.
  const withoutTitle = raw.replace(/<title>[\s\S]*?<\/title>/g, "");
  // Inline so fill="currentColor" follows the tile, not a broken <img>.
  return withoutTitle.replace(
    "<svg",
    '<svg class="agent-brand-img" focusable="false" aria-hidden="true"',
  );
}

export function agentBrandIconHtml(runtimeId: string): string {
  return iconSvg(runtimeId) ?? "";
}

export function setAgentBrandIcon(el: HTMLElement, runtimeId: string): void {
  el.classList.add("agent-brand-icon");
  const svg = iconSvg(runtimeId);
  if (svg) {
    el.innerHTML = svg;
    return;
  }
  el.textContent = (runtimeId.charAt(0) || "?").toUpperCase();
}
