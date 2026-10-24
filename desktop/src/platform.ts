/** Marks the page so CSS can use Windows fonts and spacing. */
export function markDesktopOs(): void {
  const ua = navigator.userAgent;
  const os = /\bWindows\b/.test(ua) ? "windows" : /Mac/i.test(ua) ? "macos" : "other";
  document.documentElement.dataset.os = os;
}

markDesktopOs();
