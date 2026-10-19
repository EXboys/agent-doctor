const REPO = "EXboys/agent-doctor";
const RELEASES_PAGE = `https://github.com/${REPO}/releases/latest`;

const SLOT_COPY = {
  "macos-arm64": {
    title: "macOS",
    detail: "Apple silicon disk image. Open it and drag Agent Doctor into Applications.",
    button: "Download for macOS",
  },
  "windows-x64": {
    title: "Windows",
    detail: "64-bit installer. Run it and follow the steps on screen.",
    button: "Download for Windows",
  },
  "linux-appimage": {
    title: "Linux",
    detail: "AppImage. After it downloads, allow the file to run, then open it.",
    button: "Download for Linux",
  },
};

function classify(name) {
  const n = name.toLowerCase();
  if (n.includes("team") || n.endsWith(".sig") || n.endsWith(".json")) return null;
  if (n.endsWith(".dmg")) {
    if (n.includes("x64") || n.includes("x86_64")) return "macos-x64";
    return "macos-arm64";
  }
  if (n.endsWith("setup.exe") && !n.includes("cli")) return "windows-x64";
  if (n.endsWith(".appimage")) return "linux-appimage";
  if (n.endsWith(".deb")) return "linux-deb";
  return null;
}

function thisComputer() {
  const ua = navigator.userAgent || "";
  const plat = `${navigator.platform || ""} ${ua}`;
  if (/Win/i.test(plat)) return "windows-x64";
  if (/Linux/i.test(plat) && !/Android/i.test(ua)) return "linux-appimage";
  if (/Mac/i.test(plat)) return "macos-arm64";
  return "macos-arm64";
}

function formatSize(bytes) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const mb = bytes / (1024 * 1024);
  return `${mb.toFixed(mb >= 10 ? 0 : 1)} MB`;
}

function setStatus(text) {
  const el = document.querySelector("#release-meta");
  if (el) el.textContent = text;
}

async function fetchRelease() {
  const latest = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`);
  if (latest.ok) {
    const data = await latest.json();
    if ((data.assets || []).some((asset) => classify(asset.name))) return data;
  }
  const recent = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=8`);
  if (!recent.ok) throw new Error("release list failed");
  const list = await recent.json();
  return list.find((release) => (release.assets || []).some((asset) => classify(asset.name))) || null;
}

function fillCard(slot, asset, yours) {
  const card = document.querySelector(`[data-slot="${slot}"]`);
  if (!card || !asset) return;
  card.classList.toggle("is-yours", yours);
  const file = card.querySelector(".file");
  const link = card.querySelector("[data-download]");
  const size = formatSize(asset.size);
  if (file) file.textContent = size ? `${asset.name} · ${size}` : asset.name;
  if (link) {
    link.href = asset.browser_download_url;
    link.textContent = SLOT_COPY[slot].button;
  }
}

function pointHero(asset, label) {
  const hero = document.querySelector("#hero-download");
  const nav = document.querySelector("#nav-download");
  if (!asset) return;
  for (const node of [hero, nav]) {
    if (!node) continue;
    node.href = asset.browser_download_url;
    node.textContent = label;
  }
}

async function main() {
  const header = document.querySelector(".site-header");
  const onScroll = () => header?.classList.toggle("is-stuck", window.scrollY > 4);
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });

  const mine = thisComputer();
  document.querySelector(`[data-slot="${mine}"]`)?.classList.add("is-yours");
  const hero = document.querySelector("#hero-download");
  if (hero && SLOT_COPY[mine]) hero.textContent = SLOT_COPY[mine].button;

  try {
    const release = await fetchRelease();
    if (!release) {
      setStatus("Open the releases page to pick an installer.");
      return;
    }
    const tag = release.tag_name || "";
    const assets = release.assets || [];
    const bySlot = {};
    for (const asset of assets) {
      const slot = classify(asset.name);
      if (slot && !bySlot[slot]) bySlot[slot] = asset;
    }
    setStatus(tag ? `Current release ${tag}` : "Current release");
    for (const slot of Object.keys(SLOT_COPY)) {
      fillCard(slot, bySlot[slot], slot === mine);
      if (!bySlot[slot]) {
        const file = document.querySelector(`[data-slot="${slot}"] .file`);
        if (file) {
          file.textContent = "No installer in this release yet.";
          file.classList.add("missing");
        }
      }
    }
    const deb = bySlot["linux-deb"];
    const debLink = document.querySelector("#linux-deb");
    if (deb && debLink) {
      debLink.hidden = false;
      debLink.href = deb.browser_download_url;
      debLink.textContent = "Debian package";
    }
    const intel = bySlot["macos-x64"];
    const intelLink = document.querySelector("#mac-intel");
    if (intel && intelLink) {
      intelLink.hidden = false;
      intelLink.href = intel.browser_download_url;
    }
    pointHero(bySlot[mine] || bySlot["macos-arm64"] || bySlot["windows-x64"], SLOT_COPY[mine]?.button || "Download");
  } catch {
    setStatus("The release list did not load. Use the releases page.");
  }
}

main();
