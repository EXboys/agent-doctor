const grid = document.querySelector("#skill-grid");
const filters = [...document.querySelectorAll(".filter")];

function card(item) {
  const article = document.createElement("article");
  article.className = "skill-card";
  article.dataset.group = item.group;
  article.innerHTML = `
    <div class="skill-top">
      <span class="skill-zh"></span>
      <span class="pill">Free</span>
    </div>
    <h2></h2>
    <p class="skill-summary"></p>
    <p class="skill-fits"></p>
    <p class="skill-by"></p>
    <div class="skill-actions">
      <a class="btn btn-primary" data-download href="">Download zip</a>
      <a class="skill-source" data-source href="" target="_blank" rel="noopener noreferrer">Source</a>
    </div>
  `;
  article.querySelector(".skill-zh").textContent = item.zh || "";
  article.querySelector("h2").textContent = item.name || item.id;
  article.querySelector(".skill-summary").textContent = item.summary || "";
  article.querySelector(".skill-fits").textContent = item.fits || "";
  const by = [item.author, [item.skill, item.license].filter(Boolean).join(" · "), item.installs]
    .filter(Boolean);
  article.querySelector(".skill-by").textContent = by.join("\n");
  const link = article.querySelector("[data-download]");
  link.href = item.download;
  link.download = "";
  const source = article.querySelector("[data-source]");
  source.href = item.source || item.download;
  return article;
}

function apply(group) {
  for (const button of filters) {
    const on = button.dataset.group === group;
    button.classList.toggle("on", on);
    button.setAttribute("aria-pressed", on ? "true" : "false");
  }
  for (const item of grid.querySelectorAll(".skill-card")) {
    item.hidden = group !== "all" && item.dataset.group !== group;
  }
}

async function main() {
  const header = document.querySelector(".site-header");
  const onScroll = () => header?.classList.toggle("is-stuck", window.scrollY > 4);
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });

  for (const button of filters) {
    button.addEventListener("click", () => apply(button.dataset.group || "all"));
  }

  try {
    const response = await fetch("/skills/catalog.json");
    if (!response.ok) throw new Error("catalog failed");
    const catalog = await response.json();
    const items = Array.isArray(catalog.items) ? catalog.items : [];
    grid.replaceChildren(...items.map(card));
    const bundle = catalog.bundle;
    const bundleLink = document.querySelector("#bundle-download");
    if (bundle && bundleLink && bundle.download) bundleLink.href = bundle.download;
    if (items.length === 0) {
      grid.textContent = "No skills are listed yet.";
    }
  } catch {
    grid.textContent = "The skill list did not load. Refresh the page.";
  }
}

main();
