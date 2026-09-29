/** Official brand marks. Filename (without .svg) is the runtime id. */
const iconModules = import.meta.glob("./assets/agent-icons/*.svg", {
  query: "?url",
  import: "default",
  eager: true,
}) as Record<string, string>;

function iconUrl(runtimeId: string): string | undefined {
  return iconModules[`./assets/agent-icons/${runtimeId}.svg`];
}

function iconImg(src: string): string {
  return `<img class="agent-brand-img" src="${src}" alt="" draggable="false" />`;
}

export function agentBrandIconHtml(runtimeId: string): string {
  const src = iconUrl(runtimeId);
  return src ? iconImg(src) : "";
}

export function setAgentBrandIcon(el: HTMLElement, runtimeId: string): void {
  const src = iconUrl(runtimeId);
  el.classList.add("agent-brand-icon");
  if (src) {
    el.innerHTML = iconImg(src);
    return;
  }
  el.textContent = (runtimeId.charAt(0) || "?").toUpperCase();
}
