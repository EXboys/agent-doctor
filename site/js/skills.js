const grid = document.querySelector("#skill-grid");
const filterBar = document.querySelector("#skill-filters");

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
  const by = [item.author, item.skill, item.installs].filter(Boolean);
  article.querySelector(".skill-by").textContent = by.join("\n");
  const link = article.querySelector("[data-download]");
  link.href = item.download;
  const source = article.querySelector("[data-source]");
  source.href = item.source || item.download;
  return article;
}

function apply(group) {
  for (const button of filterBar.querySelectorAll(".filter")) {
    const on = button.dataset.group === group;
    button.classList.toggle("on", on);
    button.setAttribute("aria-pressed", on ? "true" : "false");
  }
  for (const item of grid.querySelectorAll(".skill-card")) {
    item.hidden = group !== "all" && item.dataset.group !== group;
  }
}

function filterButton(group, label, on) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = on ? "filter on" : "filter";
  button.dataset.group = group;
  button.setAttribute("aria-pressed", on ? "true" : "false");
  button.textContent = label;
  button.addEventListener("click", () => apply(group));
  return button;
}

async function main() {
  const header = document.querySelector(".site-header");
  const onScroll = () => header?.classList.toggle("is-stuck", window.scrollY > 4);
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });

  try {
    const response = await fetch("/skills/catalog.json");
    if (!response.ok) throw new Error("catalog failed");
    const catalog = await response.json();
    const items = Array.isArray(catalog.items) ? catalog.items : [];
    const groups = Array.isArray(catalog.groups) ? catalog.groups : [];
    filterBar.replaceChildren(
      filterButton("all", "All", true),
      ...groups.map((group) => filterButton(group.id, group.label, false)),
    );
    grid.replaceChildren(...items.map(card));
    if (items.length === 0) {
      grid.textContent = "No skills are listed yet.";
    }
  } catch {
    grid.textContent = "The skill list did not load. Refresh the page.";
  }
}

main();
