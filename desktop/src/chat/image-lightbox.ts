import { t } from "../i18n";

const PICTURE =
  "#chat-files-image img, .chat-bubble-attachment-image, .chat-files-rendered-doc img, .diagnose-deep-attach-image";

/** Click a picture in the chat or file view to see it large, in the middle. */
export function installImageLightbox(): void {
  const box = document.createElement("div");
  box.className = "chat-image-lightbox";
  box.hidden = true;
  box.setAttribute("role", "dialog");
  box.setAttribute("aria-modal", "true");

  const close = document.createElement("button");
  close.type = "button";
  close.className = "chat-image-lightbox-close";
  close.textContent = t("chat.imageClose");

  const img = document.createElement("img");
  img.alt = "";

  box.append(close, img);
  document.body.append(box);

  const open = (source: HTMLImageElement) => {
    if (!source.getAttribute("src")) return;
    img.src = source.src;
    img.alt = source.alt || t("chat.imageZoom");
    box.setAttribute("aria-label", img.alt);
    box.hidden = false;
    close.focus();
  };

  const shut = () => {
    box.hidden = true;
    img.removeAttribute("src");
  };

  document.addEventListener("click", (event) => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const picture = target.closest("img");
    if (!(picture instanceof HTMLImageElement) || !picture.matches(PICTURE)) return;
    event.preventDefault();
    open(picture);
  });

  document.addEventListener("keydown", (event) => {
    if (box.hidden) return;
    if (event.key === "Escape") {
      event.preventDefault();
      shut();
    }
  });

  box.addEventListener("click", (event) => {
    if (event.target === box) shut();
  });
  close.addEventListener("click", shut);
}
