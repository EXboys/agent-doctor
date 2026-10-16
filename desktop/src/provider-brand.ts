/** Provider marks. Filename (without .svg) is the preset id, plus `official`. */
const iconModules = import.meta.glob("./assets/provider-icons/*.svg", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

function iconSvg(presetId: string): string | undefined {
  const raw = iconModules[`./assets/provider-icons/${presetId}.svg`];
  if (!raw) return undefined;
  const withoutTitle = raw.replace(/<title>[\s\S]*?<\/title>/g, "");
  return withoutTitle.replace(
    "<svg",
    '<svg class="agent-brand-img" focusable="false" aria-hidden="true"',
  );
}

/** Paint the official mark when we have one. Returns false so the caller can keep a letter. */
export function setProviderBrandIcon(el: HTMLElement, presetId: string): boolean {
  const svg = iconSvg(presetId);
  if (!svg) return false;
  el.classList.add("is-brand");
  el.dataset.brand = presetId;
  el.innerHTML = svg;
  return true;
}
