const grid = document.querySelector("#skill-grid");
const filterBar = document.querySelector("#skill-filters");
const searchBox = document.querySelector("#skill-search");
const clearButton = document.querySelector(".search-clear");
const countLine = document.querySelector("#skill-count");
const emptyLine = document.querySelector("#skill-empty");

let group = "all";
let query = "";

function card(item) {
  const article = document.createElement("article");
  article.className = "skill-card";
  article.dataset.group = item.group;
  article.dataset.search = [item.name, item.zh, item.summary, item.fits, item.author, item.skill]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
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

function apply() {
  const needle = query.trim().toLowerCase();
  const cards = grid.querySelectorAll(".skill-card");
  let shown = 0;
  for (const item of cards) {
    const inGroup = group === "all" || item.dataset.group === group;
    const matches = !needle || (item.dataset.search || "").includes(needle);
    item.hidden = !(inGroup && matches);
    if (!item.hidden) shown += 1;
  }
  for (const button of filterBar.querySelectorAll(".filter")) {
    const on = button.dataset.group === group;
    button.classList.toggle("on", on);
    button.setAttribute("aria-pressed", on ? "true" : "false");
  }
  if (countLine) {
    const noun = shown === 1 ? "skill" : "skills";
    countLine.textContent = needle ? `${shown} ${noun} match “${query.trim()}”` : `${shown} ${noun}`;
  }
  if (emptyLine) emptyLine.hidden = shown !== 0 || cards.length === 0;
}

function filterButton(id, label, on) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = on ? "filter on" : "filter";
  button.dataset.group = id;
  button.setAttribute("aria-pressed", on ? "true" : "false");
  button.textContent = label;
  button.addEventListener("click", () => {
    group = id;
    apply();
  });
  return button;
}

function resetSearch() {
  query = "";
  if (searchBox) searchBox.value = "";
  if (clearButton) clearButton.hidden = true;
  apply();
}

if (searchBox) {
  searchBox.addEventListener("input", () => {
    query = searchBox.value;
    if (clearButton) clearButton.hidden = !query;
    apply();
  });
  searchBox.addEventListener("keydown", (event) => {
    if (event.key === "Escape") resetSearch();
  });
}

if (clearButton) {
  clearButton.addEventListener("click", () => {
    resetSearch();
    searchBox?.focus();
  });
}

async function main() {
  try {
    const response = await fetch("/skills/catalog.json");
    if (!response.ok) throw new Error("catalog failed");
    const catalog = await response.json();
    const items = Array.isArray(catalog.items) ? catalog.items : [];
    const groups = Array.isArray(catalog.groups) ? catalog.groups : [];
    filterBar.replaceChildren(
      filterButton("all", "All", true),
      ...groups.map((entry) => filterButton(entry.id, entry.label, false)),
    );
    grid.replaceChildren(...items.map(card));
    if (items.length === 0) {
      grid.textContent = "No skills are listed yet.";
    }
    apply();
  } catch {
    grid.textContent = "The skill list did not load. Refresh the page.";
  }
}

main();
