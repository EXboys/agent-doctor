/** Drive the preview page: click, fill, and check that words showed up. */

const RUNNER = `<script>
(function () {
  function visible(el) {
    if (!el || !el.getBoundingClientRect) return false;
    var style = window.getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden") return false;
    var box = el.getBoundingClientRect();
    return box.width > 0 && box.height > 0;
  }
  function words(el) {
    return String(
      el.getAttribute("aria-label") ||
      el.getAttribute("placeholder") ||
      el.innerText ||
      el.textContent ||
      el.value ||
      ""
    ).replace(/\\s+/g, " ").trim();
  }
  function findClick(query) {
    var nodes = document.querySelectorAll("button, a, summary, label, [role='button'], input[type='button'], input[type='submit']");
    var list = Array.prototype.filter.call(nodes, visible);
    var exact = list.filter(function (el) { return words(el) === query; })[0];
    if (exact) return exact;
    return list.filter(function (el) { return words(el).indexOf(query) >= 0; })[0] || null;
  }
  function findField(query) {
    var nodes = document.querySelectorAll("input:not([type='hidden']):not([type='button']):not([type='submit']):not([type='checkbox']):not([type='radio']), textarea");
    var list = Array.prototype.filter.call(nodes, visible);
    function blob(el) {
      var bits = [el.getAttribute("placeholder"), el.getAttribute("aria-label"), el.getAttribute("name"), el.id];
      if (el.id) {
        var lab = document.querySelector("label[for='" + CSS.escape(el.id) + "']");
        if (lab) bits.push(lab.innerText || "");
      }
      return bits.filter(Boolean).join(" ");
    }
    var exact = list.filter(function (el) {
      return el.getAttribute("placeholder") === query || el.getAttribute("aria-label") === query;
    })[0];
    if (exact) return exact;
    return list.filter(function (el) { return blob(el).indexOf(query) >= 0; })[0] || null;
  }
  window.addEventListener("message", function (event) {
    var data = event.data;
    if (!data || data.source !== "agent-doctor-ui" || !data.step) return;
    var step = data.step;
    var query = String(step.target || "").trim();
    var code = "failed";
    var ok = false;
    if (step.kind === "click") {
      var hit = findClick(query);
      if (hit) { hit.click(); ok = true; code = "clicked"; }
      else code = "missing";
    } else if (step.kind === "fill") {
      var field = findField(query);
      if (!field) code = "nofield";
      else {
        field.focus();
        field.value = String(step.text || "");
        field.dispatchEvent(new Event("input", { bubbles: true }));
        field.dispatchEvent(new Event("change", { bubbles: true }));
        ok = true;
        code = "filled";
      }
    } else if (step.kind === "see") {
      var body = (document.body && (document.body.innerText || document.body.textContent) || "").replace(/\\s+/g, " ");
      ok = query.length > 0 && body.indexOf(query) >= 0;
      code = ok ? "seen" : "unseen";
    }
    parent.postMessage({ source: "agent-doctor-ui-result", id: data.id, ok: ok, code: code, target: query }, "*");
  });
})();
</script>`;

export type UiStepKind = "click" | "fill" | "see";

export type UiStep = {
  kind: UiStepKind;
  /** Button label, field hint, or words that should appear. */
  target: string;
  /** Words to type, only for fill. */
  text: string;
};

export function wrapHtmlForPreview(html: string): string {
  if (/<head[\s>]/i.test(html)) return html.replace(/<head([^>]*)>/i, `<head$1>${RUNNER}`);
  return `${RUNNER}${html}`;
}

/** A web address, or a short local one like localhost:3000. */
export function pageAddress(raw: string): string | null {
  const text = raw.trim();
  if (!text) return null;
  if (/^https?:\/\//i.test(text)) return text;
  if (/^[\w.-]+(:\d+)?(\/.*)?$/.test(text) && /[.:]/.test(text)) return `http://${text}`;
  return null;
}
