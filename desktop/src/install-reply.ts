import { t } from "./i18n";
import { submitInstallInput } from "./ipc";

/** One-line reply, or a collapsed typing area for a back-and-forth install. */
export function mountInstallReply(host: HTMLElement, kind: string): void {
  const existing = host.querySelector<HTMLElement>("[data-install-reply]");
  if (!kind) {
    existing?.remove();
    return;
  }
  if (existing?.dataset.kind === kind) {
    return;
  }
  existing?.remove();

  const secret = kind === "secret";
  const session = kind === "session";
  const box = document.createElement("div");
  box.className = "install-reply";
  box.dataset.installReply = "1";
  box.dataset.kind = kind;

  const form = document.createElement("form");
  form.className = "install-reply-form";
  form.hidden = session;

  const label = document.createElement("span");
  label.className = "install-reply-label";
  label.textContent = secret ? t("chat.pasteSecret") : t("chat.replyHere");

  const field = document.createElement("input");
  field.type = secret ? "password" : "text";
  field.autocomplete = "off";
  field.spellcheck = false;
  field.setAttribute("aria-label", label.textContent);

  const button = document.createElement("button");
  button.type = "submit";
  button.className = "btn-primary btn-compact";
  button.textContent = t("chat.inputSubmit");

  const note = document.createElement("p");
  note.className = "install-reply-note";
  note.hidden = true;

  form.append(label, field, button, note);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const text = field.value.trim();
    if (!text) {
      return;
    }
    button.disabled = true;
    void submitInstallInput(text)
      .then(() => {
        field.value = "";
        button.disabled = false;
        note.hidden = true;
        if (session) {
          form.hidden = true;
          const open = box.querySelector<HTMLButtonElement>("[data-install-reply-open]");
          if (open) open.hidden = false;
        }
      })
      .catch(() => {
        button.disabled = false;
        note.hidden = false;
        note.textContent = t("chat.inputFailed");
      });
  });

  if (session) {
    const open = document.createElement("button");
    open.type = "button";
    open.className = "btn-ghost btn-compact";
    open.dataset.installReplyOpen = "1";
    open.textContent = t("chat.typeHere");
    open.addEventListener("click", () => {
      form.hidden = false;
      open.hidden = true;
      field.focus();
    });
    box.append(open, form);
  } else {
    box.append(form);
  }
  host.append(box);
}
