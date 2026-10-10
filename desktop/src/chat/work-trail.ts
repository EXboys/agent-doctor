import { t } from "../i18n";

/** One quiet row for a turn's thinking and tool steps. The notes stay behind the click. */
export function createWorkTrail(live: boolean): HTMLDetailsElement {
  const trail = document.createElement("details");
  trail.className = live ? "chat-work is-live" : "chat-work";
  trail.open = false;
  trail.innerHTML = `
    <summary class="chat-work-summary">
      <span class="chat-work-label"></span>
      <span class="chat-activity-elapsed"></span>
      <span class="chat-work-chevron" aria-hidden="true"></span>
    </summary>
    <div class="chat-work-body"></div>
  `;
  const label = trail.querySelector<HTMLElement>(".chat-work-label");
  if (label) label.textContent = live ? t("chat.workLive") : t("chat.workDone");
  trail.addEventListener("toggle", () => {
    if (trail.open) trail.dataset.pinned = "1";
    else delete trail.dataset.pinned;
  });
  return trail;
}

export function wrapProcessRows(rows: HTMLElement[], live = false): HTMLElement {
  if (rows.length === 1) return rows[0];
  const trail = createWorkTrail(live);
  trail.querySelector(".chat-work-body")!.append(...rows);
  return trail;
}

function isLooseProcess(el: Element | null): boolean {
  if (!(el instanceof HTMLElement)) return false;
  if (el.classList.contains("chat-thinking")) return true;
  return (
    el.classList.contains("chat-tool-group") &&
    !el.classList.contains("chat-permission-group") &&
    !el.classList.contains("chat-turn-tools")
  );
}

/** Process rows sitting under the latest answer, skipping a fleeting status line. */
function trailingProcess(logEl: HTMLElement): HTMLElement[] {
  const found: HTMLElement[] = [];
  let el = logEl.lastElementChild as HTMLElement | null;
  while (el) {
    if (el.classList.contains("chat-work") || isLooseProcess(el)) {
      found.unshift(el);
      el = el.previousElementSibling as HTMLElement | null;
      continue;
    }
    if (el.classList.contains("chat-activity") && !el.classList.contains("chat-tool-row")) {
      el = el.previousElementSibling as HTMLElement | null;
      continue;
    }
    break;
  }
  return found;
}

function markTrailLive(trail: HTMLElement): void {
  trail.classList.add("is-live");
  const label = trail.querySelector<HTMLElement>(".chat-work-label");
  if (label) label.textContent = t("chat.workLive");
}

/** Keep a single status line. The first step stays itself; the next ones fold in. */
export function placeProcessBlock(logEl: HTMLElement, node: HTMLElement): void {
  const trailing = trailingProcess(logEl);
  if (trailing.length === 0) {
    logEl.appendChild(node);
    return;
  }
  let trail = trailing.find((el) => el.classList.contains("chat-work"));
  if (!trail) {
    trail = createWorkTrail(true);
    trailing[0].replaceWith(trail);
  }
  const body = trail.querySelector(".chat-work-body");
  if (!body) {
    logEl.appendChild(node);
    return;
  }
  for (const el of trailing) {
    if (el === trail) continue;
    if (el.classList.contains("chat-work")) {
      const nested = el.querySelector(".chat-work-body");
      if (nested) body.append(...Array.from(nested.childNodes));
      el.remove();
      continue;
    }
    body.appendChild(el);
  }
  body.appendChild(node);
  markTrailLive(trail);
}

export function sealWorkTrail(logEl: HTMLElement): void {
  for (const trail of logEl.querySelectorAll<HTMLDetailsElement>(":scope > .chat-work.is-live")) {
    trail.classList.remove("is-live");
    const elapsed = trail.querySelector(".chat-activity-elapsed")?.textContent?.trim() ?? "";
    const label = trail.querySelector<HTMLElement>(".chat-work-label");
    if (label) label.textContent = elapsed ? t("chat.workDoneElapsed", { elapsed }) : t("chat.workDone");
    const elapsedEl = trail.querySelector<HTMLElement>(".chat-activity-elapsed");
    if (elapsedEl) elapsedEl.textContent = "";
    if (trail.dataset.pinned !== "1") trail.open = false;
  }
}

/** The open thinking block sits inside the one status row, so that row stays live too. */
export function noteThinkingLive(block: HTMLElement): void {
  const trail = block.closest<HTMLElement>(".chat-work");
  if (!trail) return;
  markTrailLive(trail);
}
