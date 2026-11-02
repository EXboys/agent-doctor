// Shared header behaviour: the sticky hairline and the small-screen menu.
(() => {
  const header = document.querySelector(".site-header");
  const toggle = document.querySelector(".nav-toggle");
  const menu = document.getElementById("nav-menu");

  const onScroll = () => header?.classList.toggle("is-stuck", window.scrollY > 4);
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });

  // The header height changes between breakpoints, so the sticky bar under it
  // reads the real number instead of a guess.
  const syncHeaderHeight = () => {
    const height = header?.offsetHeight;
    if (height) document.documentElement.style.setProperty("--header-h", `${height}px`);
  };
  syncHeaderHeight();
  window.addEventListener("resize", syncHeaderHeight);

  const setMenu = (open) => {
    if (!toggle || !menu) return;
    toggle.setAttribute("aria-expanded", open ? "true" : "false");
    toggle.setAttribute("aria-label", open ? "Close the menu" : "Open the menu");
    menu.classList.toggle("is-open", open);
  };

  if (!toggle || !menu) return;

  toggle.addEventListener("click", () => {
    setMenu(toggle.getAttribute("aria-expanded") !== "true");
  });

  menu.addEventListener("click", (event) => {
    if (event.target.closest("a")) setMenu(false);
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") setMenu(false);
  });

  // The inline links come back on a wide screen, so the panel has no job left.
  window.matchMedia("(min-width: 981px)").addEventListener("change", (event) => {
    if (event.matches) setMenu(false);
  });
})();
