/** Full-page overlays inside #chat-main (knowledge, settings). */

export function isKnowledgePageOpen(): boolean {
  const page = document.querySelector<HTMLElement>("#chat-knowledge");
  if (page && !page.hidden) return true;
  return (
    document.querySelector("#chat-main")?.classList.contains("is-knowledge") === true ||
    document.querySelector("#chat-shell")?.classList.contains("is-knowledge") === true
  );
}

export function closeKnowledgePage(): void {
  document.querySelector("#chat-shell")?.classList.remove("is-knowledge");
  document.querySelector("#chat-main")?.classList.remove("is-knowledge");
  const page = document.querySelector<HTMLElement>("#chat-knowledge");
  if (page) {
    page.hidden = true;
    page.setAttribute("aria-hidden", "true");
  }
  const open = document.querySelector<HTMLButtonElement>("#chat-knowledge-open");
  open?.classList.remove("is-on");
  open?.setAttribute("aria-pressed", "false");
}

export function isChatSettingsPageOpen(): boolean {
  const page = document.querySelector<HTMLElement>("#chat-settings");
  if (page && !page.hidden) return true;
  return document.querySelector("#chat-main")?.classList.contains("is-settings") === true;
}

export function closeChatSettingsPage(): void {
  document.querySelector("#chat-shell")?.classList.remove("is-settings");
  document.querySelector("#chat-main")?.classList.remove("is-settings");
  const page = document.querySelector<HTMLElement>("#chat-settings");
  if (page) {
    page.hidden = true;
    page.setAttribute("aria-hidden", "true");
  }
  const open = document.querySelector<HTMLButtonElement>("#chat-settings-open");
  open?.classList.remove("is-on");
  open?.setAttribute("aria-pressed", "false");
}

export function isResourcesMainPageOpen(): boolean {
  const page = document.querySelector<HTMLElement>("#chat-resources-page");
  if (page && !page.hidden) return true;
  return document.querySelector("#chat-main")?.classList.contains("is-resources-page") === true;
}

export function closeResourcesMainPage(): void {
  document.querySelector("#chat-shell")?.classList.remove("is-resources-page");
  document.querySelector("#chat-main")?.classList.remove("is-resources-page");
  const page = document.querySelector<HTMLElement>("#chat-resources-page");
  if (page) {
    page.hidden = true;
    page.setAttribute("aria-hidden", "true");
  }
  const open = document.querySelector<HTMLButtonElement>("#chat-resources-toggle");
  open?.classList.remove("is-on");
  open?.setAttribute("aria-pressed", "false");
}

export function isSchedulePageOpen(): boolean {
  const page = document.querySelector<HTMLElement>("#chat-schedule");
  if (page && !page.hidden) return true;
  return document.querySelector("#chat-main")?.classList.contains("is-schedule") === true;
}

export function closeSchedulePage(): void {
  document.querySelector("#chat-shell")?.classList.remove("is-schedule");
  document.querySelector("#chat-main")?.classList.remove("is-schedule");
  const page = document.querySelector<HTMLElement>("#chat-schedule");
  if (page) {
    page.hidden = true;
    page.setAttribute("aria-hidden", "true");
  }
  const open = document.querySelector<HTMLButtonElement>("#chat-schedule-open");
  open?.classList.remove("is-on");
  open?.setAttribute("aria-pressed", "false");
}

/** Leave knowledge / settings / resources / schedule so the chat transcript is visible again. */
export function leaveChatOverlayPages(): boolean {
  const hadOverlay =
    isKnowledgePageOpen() ||
    isChatSettingsPageOpen() ||
    isResourcesMainPageOpen() ||
    isSchedulePageOpen();
  if (isKnowledgePageOpen()) closeKnowledgePage();
  if (isChatSettingsPageOpen()) closeChatSettingsPage();
  if (isResourcesMainPageOpen()) closeResourcesMainPage();
  if (isSchedulePageOpen()) closeSchedulePage();
  return hadOverlay;
}
