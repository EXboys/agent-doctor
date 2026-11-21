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
    detail: "64-bit installer for Windows 10 or newer. Run it and follow the steps on screen.",
    button: "Download for Windows",
  },
  "linux-appimage": {
    title: "Linux",
    detail: "AppImage. After it downloads, allow the file to run, then open it.",
    button: "Download for Linux",
  },
};

const TEAM_OSS = "https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop-team";

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

function classifyTeam(name) {
  const n = name.toLowerCase();
  if (!n.includes("team") || n.endsWith(".sig") || n.endsWith(".json")) return null;
  if (n.endsWith(".dmg")) {
    if (n.includes("x64") || n.includes("x86_64")) return "macos-x64";
    return "macos-arm64";
  }
  if (n.endsWith("setup.exe") && !n.includes("cli")) return "windows-x64";
  return null;
}

function teamOssUrl(tag, fileName) {
  const channel = /-beta\./i.test(tag) ? "/beta" : "";
  return `${TEAM_OSS}${channel}/${fileName}`;
}

/** [major, minor, patch, betaN]. A plain release sorts after its own betas. */
function versionKey(tag) {
  const match = String(tag)
    .replace(/^v/, "")
    .match(/^(\d+)\.(\d+)\.(\d+)(?:-beta\.(\d+))?$/);
  if (!match) return null;
  return [
    Number(match[1]),
    Number(match[2]),
    Number(match[3]),
    match[4] == null ? Number.POSITIVE_INFINITY : Number(match[4]),
  ];
}

function newerTag(left, right) {
  const a = versionKey(left);
  const b = versionKey(right);
  if (!a) return false;
  if (!b) return true;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) return a[i] > b[i];
  }
  return false;
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
  if (hero) {
    hero.href = asset.browser_download_url;
    hero.textContent = label;
  }
  // The header button keeps a short label. "Download for macOS" is wider than
  // a phone and pushes the nav off the screen.
  if (nav) nav.href = asset.browser_download_url;
}

async function main() {
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
    await loadTeamDownload(mine);
  } catch {
    setStatus("The release list did not load. Use the releases page.");
    await loadTeamDownload(mine);
  }
}

async function loadTeamDownload(mine) {
  const meta = document.querySelector("#team-release-meta");
  const setMeta = (text) => {
    if (meta) meta.textContent = text;
  };
  try {
    const recent = await fetch(`https://api.github.com/repos/${REPO}/releases?per_page=20`);
    if (!recent.ok) throw new Error("release list failed");
    const list = await recent.json();
    const release = list
      .filter((item) => (item.assets || []).some((asset) => classifyTeam(asset.name)))
      .sort((a, b) => (newerTag(a.tag_name, b.tag_name) ? -1 : 1))[0];
    if (!release) {
      setMeta("No team installer in the recent releases.");
      return;
    }
    const tag = release.tag_name || "";
    const bySlot = {};
    for (const asset of release.assets || []) {
      const slot = classifyTeam(asset.name);
      if (slot && !bySlot[slot]) bySlot[slot] = asset;
    }
    const test = /-beta\./i.test(tag);
    setMeta(tag ? `${test ? "Test version " : ""}${tag}` : "Team release");
    for (const slot of ["macos-arm64", "windows-x64"]) {
      const card = document.querySelector(`[data-slot="team-${slot}"]`);
      const asset = bySlot[slot];
      if (!card) continue;
      card.classList.toggle("is-yours", slot === mine);
      const file = card.querySelector(".file");
      const link = card.querySelector("[data-team-download]");
      if (!asset) {
        if (file) file.textContent = "No installer in this release yet.";
        continue;
      }
      const size = formatSize(asset.size);
      if (file) file.textContent = size ? `${asset.name} · ${size}` : asset.name;
      if (link) link.href = teamOssUrl(tag, asset.name);
    }
  } catch {
    setMeta("The team download did not load. Use the releases page.");
  }
}

main();
