import { listen } from "@tauri-apps/api/event";
import { escapeHtml } from "./format";
import { applyStaticI18n, t } from "./i18n";
import { llamaFileSize, llamaModelCopy } from "./llama-models";
import { renderBoardTopology, type BoardDevice, type BoardDeviceKind } from "../../vendor/board-ui/topology";
import {
  activateClusterModel,
  cancelClusterInstall,
  getClusterStatus,
  setClusterModelSource,
  openAskWindow,
  scanLlamaPeers,
  setLlamaPeerHosts,
  startLocalCluster,
} from "./ipc";
import { mountWindowChrome } from "./window-chrome";
import type { ClusterInstallProgress, ClusterModelTier, ClusterStatus } from "./types";

type BoardTab = "recommend" | "download" | "network";

const tabs: BoardTab[] = ["recommend", "download", "network"];

let tab: BoardTab = "recommend";
let status: ClusterStatus | null = null;
let pendingModelId: string | null = null;
let busy: "idle" | "installing" | "preparing" = "idle";
let progress: ClusterInstallProgress | null = null;
let peersDraft = "";
let addDraft = "";
let addingPeer = false;
let scanning = false;
let foundPeers: string[] = [];
let errorText = "";

function tabLabel(id: BoardTab): string {
  if (id === "recommend") return t("cluster.boardRecommend");
  if (id === "download") return t("cluster.boardDownload");
  return t("cluster.boardNetwork");
}

function chosenModel(): string {
  if (!status) return "";
  if (pendingModelId && status.tiers.some((tier) => tier.model_id === pendingModelId)) {
    return pendingModelId;
  }
  return status.selected_model;
}

function chosenTier(): ClusterModelTier | undefined {
  const id = chosenModel();
  return status?.tiers.find((tier) => tier.model_id === id);
}

const TAB_ICON: Record<BoardTab, string> = {
  recommend: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/></svg>`,
  download: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3v12"/><path d="M7 12l5 5 5-5"/><path d="M5 21h14"/></svg>`,
  network: `<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/></svg>`,
};

function progressPct(): number {
  const total = progress?.total_bytes ?? 0;
  if (!progress || total <= 0) return 0;
  return Math.max(4, Math.min(100, Math.round((progress.downloaded_bytes / total) * 100)));
}

function progressSentence(): string {
  if (!progress) return "";
  const total = progress.total_bytes ?? 0;
  const done = Math.round(progress.downloaded_bytes / (1024 * 1024));
  if (progress.stage === "model" && total > 0) {
    return t("cluster.downloadingModel", {
      done: String(done),
      total: String(Math.round(total / (1024 * 1024))),
    });
  }
  if (progress.stage === "extracting") return t("cluster.extracting");
  return t("cluster.downloadingNoTotal", { done: String(done) });
}

function sourcePicker(): string {
  const source = status?.model_source === "huggingface" ? "huggingface" : "modelscope";
  const hint =
    source === "huggingface" ? t("cluster.modelSourceHfHint") : t("cluster.modelSourceChinaHint");
  const pill = (id: "modelscope" | "huggingface", label: string) =>
    `<button type="button" class="ad-source-pill${source === id ? " is-selected" : ""}" data-board-action="model-source" data-source="${id}" ${
      busy !== "idle" ? "disabled" : ""
    }>${escapeHtml(label)}</button>`;
  return `<div class="ad-source">
    <span class="ad-source-label">${escapeHtml(t("cluster.modelSourceLabel"))}</span>
    <div class="ad-source-pills">${pill("modelscope", t("cluster.modelSourceChina"))}${pill(
      "huggingface",
      t("cluster.modelSourceHf"),
    )}</div>
    <p class="ad-source-hint">${escapeHtml(hint)}</p>
  </div>`;
}

function meter(label: string, strong: string, pct: number, indeterminate = false): string {
  return `<div class="ad-meter-box">
    <div class="ad-meter-head"><span>${escapeHtml(label)}</span><strong>${escapeHtml(strong)}</strong></div>
    <div class="ad-meter-track" aria-hidden="true"><div class="ad-meter-fill${
      indeterminate ? " is-indeterminate" : ""
    }" style="width:${indeterminate ? 40 : pct}%"></div></div>
  </div>`;
}

function modelRows(tiers: ClusterModelTier[], ready: boolean): string {
  return [...tiers]
    .sort((a, b) => (b.size_bytes ?? 0) - (a.size_bytes ?? 0))
    .map((tier) => {
      const copy = llamaModelCopy(tier);
      const selected = tier.model_id === chosenModel();
      const locked = !tier.fits_cluster;
      const size = llamaFileSize(tier.size_bytes);
      const hint = tier.on_disk ? `${copy.hint} · ${t("cluster.modelOnDisk")}` : copy.hint;
      return `<li><button type="button" class="ad-model-row${ready ? " is-ready" : ""}${
        selected && ready ? " is-selected" : ""
      }" data-board-action="pick" data-model="${escapeHtml(tier.model_id)}" ${
        locked || busy !== "idle" ? "disabled" : ""
      }>
        <span class="ad-model-main">
          <strong>${escapeHtml(copy.name)}</strong>
          <span>${escapeHtml(hint)}</span>
        </span>
        ${size ? `<span class="ad-size-pill${ready ? " is-ready" : ""}">${escapeHtml(size)}</span>` : ""}
      </button></li>`;
    })
    .join("");
}

function recommendPanel(): string {
  if (!status) return "";
  const fits = status.tiers.filter((tier) => tier.fits_cluster);
  const larger = status.tiers.filter((tier) => !tier.fits_cluster);
  const tier = chosenTier();
  const wired = Boolean(status.provider_active && status.active_models.includes(chosenModel()));
  let action = "";
  if (!status.runtime_installed) {
    action = `<button type="button" class="ad-btn-pill is-primary" data-board-action="turn-on" ${
      busy !== "idle" ? "disabled" : ""
    }>${escapeHtml(t("cluster.boardTurnOn"))}</button>`;
  } else if (!wired) {
    const label = tier?.on_disk
      ? status.provider_active
        ? t("cluster.btnSwitchModel")
        : t("cluster.btnApplyTier")
      : t("cluster.btnDownloadUse");
    action = `<button type="button" class="ad-btn-pill is-primary" data-board-action="use" ${
      busy !== "idle" || (tier && !tier.fits_cluster) ? "disabled" : ""
    }>${escapeHtml(label)}</button>`;
  }
  const cancel =
    busy !== "idle"
      ? `<button type="button" class="ad-btn-pill" data-board-action="cancel">${escapeHtml(
          t("cluster.btnCancelDownload"),
        )}</button>`
      : "";
  const pct = fits.length && status.tiers.length ? Math.round((fits.length / status.tiers.length) * 100) : 0;
  return `<section class="ad-hero-card">
      <div class="ad-hero-top">
        <div>
          <h1 class="ad-hero-title">${escapeHtml(t("cluster.fitsLabel"))}</h1>
          <p class="ad-hero-sub">${escapeHtml(
            !status.runtime_installed
              ? t("cluster.boardNeedRuntime")
              : status.peer_hosts.trim()
                ? t("cluster.boardLeadTogether")
                : t("cluster.boardLead"),
          )}</p>
        </div>
        ${action}
      </div>
      ${sourcePicker()}
      ${meter(t("cluster.boardThisMac"), t("cluster.boardFitCount", { count: String(fits.length) }), pct)}
      ${wired ? `<p class="ad-callout">${escapeHtml(t("cluster.chatUsesModel"))}</p>` : ""}
      ${
        busy !== "idle"
          ? meter(
              progressSentence() ||
                (status.model_source === "huggingface"
                  ? t("cluster.preparingModelHf")
                  : t("cluster.boardDownload")),
              progressPct() ? `${progressPct()}%` : "…",
              progressPct(),
              progressPct() === 0,
            )
          : ""
      }
      ${cancel ? `<div class="ad-hero-actions">${cancel}</div>` : ""}
    </section>
    <section class="ad-section-card">
      <div class="ad-section-head">
        <h2>${escapeHtml(t("cluster.boardFitsNow"))}</h2>
        <span class="ad-badge is-ok">${fits.length}</span>
      </div>
      ${
        fits.length
          ? `<ul class="ad-model-list">${modelRows(fits, true)}</ul>`
          : `<p class="ad-empty-inline">${escapeHtml(t("cluster.boardNoneFit"))}</p>`
      }
    </section>
    ${
      larger.length
        ? `<section class="ad-section-card">
            <div class="ad-section-head">
              <div>
                <h2>${escapeHtml(t("cluster.largerModels"))}</h2>
                <p class="ad-section-note">${escapeHtml(t("cluster.boardLargerNote"))}</p>
              </div>
              <span class="ad-badge">${larger.length}</span>
            </div>
            <ul class="ad-model-list">${modelRows(larger, false)}</ul>
          </section>`
        : ""
    }
    <div class="ad-foot-link"><button type="button" data-board-tab="download">${escapeHtml(t("cluster.boardOpenDownloads"))}</button></div>`;
}

function downloadPanel(): string {
  if (!status) return "";
  const saved = status.tiers.filter((tier) => tier.on_disk);
  const rows = saved
    .map((tier) => {
      const copy = llamaModelCopy(tier);
      const active = status?.provider_active && status.active_models.includes(tier.model_id);
      return `<li class="ad-model-row is-ready">
        <span class="ad-model-main"><strong>${escapeHtml(copy.name)}</strong><span>${escapeHtml(copy.hint)}</span></span>
        ${
          active
            ? `<span class="ad-size-pill is-ready">${escapeHtml(t("cluster.badgeActive"))}</span>`
            : `<button type="button" class="ad-btn-pill" data-board-action="use-saved" data-model="${escapeHtml(
                tier.model_id,
              )}" ${busy !== "idle" ? "disabled" : ""}>${escapeHtml(t("cluster.boardUseDownloaded"))}</button>`
        }
      </li>`;
    })
    .join("");
  const cancel =
    busy !== "idle"
      ? `<button type="button" class="ad-btn-pill" data-board-action="cancel">${escapeHtml(
          t("cluster.btnCancelDownload"),
        )}</button>`
      : "";
  const pct = progressPct();
  return `<section class="ad-hero-card">
      <div class="ad-hero-top">
        <div>
          <h1 class="ad-hero-title">${escapeHtml(t("cluster.boardDownload"))}</h1>
          <p class="ad-hero-sub">${escapeHtml(
            busy !== "idle" && status?.model_source === "huggingface" && !progress
              ? t("cluster.preparingModelHf")
              : busy !== "idle" && progress
                ? progressSentence()
                : t("cluster.boardDownloadLead"),
          )}</p>
        </div>
        ${cancel}
      </div>
      ${sourcePicker()}
      ${
        busy !== "idle"
          ? meter(t("cluster.boardDownload"), pct ? `${pct}%` : "…", pct, pct === 0)
          : ""
      }
    </section>
    ${
      saved.length
        ? `<section class="ad-section-card">
            <div class="ad-section-head"><h2>${escapeHtml(t("cluster.modelOnDisk"))}</h2><span class="ad-badge is-ok">${saved.length}</span></div>
            <ul class="ad-model-list">${rows}</ul>
          </section>`
        : `<div class="ad-empty-card">${escapeHtml(t("cluster.boardEmptyDownloads"))}</div>`
    }`;
}

function normalizePeer(host: string): string {
  return host.trim().replace(/^https?:\/\//, "").replace(/:50052$/, "");
}

function peerTokens(raw: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const part of raw.split(/[,，;\s]+/)) {
    const host = normalizePeer(part);
    if (!host || seen.has(host)) continue;
    seen.add(host);
    out.push(host);
  }
  return out;
}

function networkDevices(): BoardDevice[] {
  const peers = peerTokens(peersDraft);
  const usable = llamaFileSize(status?.usable_memory_bytes);
  const selfDetail = !status?.runtime_installed
    ? t("cluster.boardNodeOff")
    : usable
      ? `${t("cluster.boardNodeOn")} · ${t("cluster.boardUsable", { size: usable })}`
      : t("cluster.boardNodeOn");
  const kind: BoardDeviceKind = document.documentElement.dataset.os === "macos" ? "mac-mini" : "macbook";
  const devices: BoardDevice[] = [
    {
      id: "local",
      name: t("cluster.boardThisMac"),
      kind,
      detail: selfDetail,
    },
    ...peers.map((host, index) => ({
      id: host,
      name: peers.length === 1 ? t("cluster.boardNodeOther") : t("cluster.boardNodeIndexed", { n: String(index + 2) }),
      kind,
      detail: foundPeers.includes(host)
        ? `${t("cluster.boardNodeFound")} · ${host}`
        : `${t("cluster.boardNodeAdded")} · ${host}`,
      dimmed: !foundPeers.includes(host),
      action: { label: t("cluster.boardRemovePeer"), peer: host },
    })),
  ];
  if (scanning && peers.length === 0) {
    devices.push({
      id: "looking",
      name: t("cluster.boardLooking"),
      kind,
      detail: t("cluster.peersOptional"),
      dimmed: true,
    });
  }
  return devices;
}

function networkPanel(): string {
  const peers = peerTokens(peersDraft);
  const hint = scanning
    ? t("cluster.llamaScanning")
    : peers.length > 0
      ? t("cluster.llamaFound", { count: String(peers.length) })
      : t("cluster.boardNetworkLead");
  const turnOn = status?.runtime_installed
    ? ""
    : `<button type="button" class="ad-btn-pill is-primary" data-board-action="turn-on" ${
        busy !== "idle" ? "disabled" : ""
      }>${escapeHtml(t("cluster.boardTurnOn"))}</button>`;
  const add = addingPeer
    ? `<input id="llama-board-peers" class="ad-field" type="text" value="${escapeHtml(
        addDraft,
      )}" placeholder="${escapeHtml(t("cluster.llamaPeersPlaceholder"))}" ${busy !== "idle" ? "disabled" : ""} />`
    : "";
  const addButton = addingPeer
    ? `<button type="button" class="ad-btn-pill is-primary" data-board-action="save-peers" ${
        busy !== "idle" ? "disabled" : ""
      }>${escapeHtml(t("cluster.boardSavePeers"))}</button>`
    : `<button type="button" class="ad-btn-pill" data-board-action="show-add" ${
        busy !== "idle" ? "disabled" : ""
      }>${escapeHtml(t("cluster.boardAddPeer"))}</button>`;
  return `<section class="ad-topo-stage">${renderBoardTopology(networkDevices())}</section>
      <p class="ad-hero-sub">${escapeHtml(hint)}</p>
      ${add}
      <div class="ad-hero-actions">
        ${turnOn}
        <button type="button" class="ad-btn-pill${status?.runtime_installed ? " is-primary" : ""}" data-board-action="scan" ${
          busy !== "idle" || !status?.runtime_installed ? "disabled" : ""
        }>${escapeHtml(t("cluster.boardScan"))}</button>
        ${addButton}
      </div>`;
}

function render() {
  const nav = document.querySelector("#llama-board-tabs");
  const main = document.querySelector("#llama-board-main");
  if (!nav || !main) return;
  const pct = progressPct();
  nav.innerHTML = tabs
    .map((id) => {
      const mark =
        id === "download" && busy !== "idle" && pct
          ? `<span class="ad-board-dl-pill">${pct}%</span>`
          : TAB_ICON[id];
      return `<button type="button" class="ad-board-tab${id === tab ? " is-active" : ""}" data-board-tab="${id}">${mark}<span>${escapeHtml(
        tabLabel(id),
      )}</span></button>`;
    })
    .join("");
  const body = tab === "download" ? downloadPanel() : tab === "network" ? networkPanel() : recommendPanel();
  const error = errorText ? `<div class="ad-empty-card is-error">${escapeHtml(errorText)}</div>` : "";
  main.innerHTML = `${error}${body}`;
}

function friendly(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err ?? "");
  return raw.replace(/^Error:\s*/i, "").trim() || t("personal.applyFailed");
}

async function pickSource(source: "modelscope" | "huggingface") {
  if (!status || status.model_source === source || busy !== "idle") return;
  try {
    status = await setClusterModelSource(source);
  } catch (err) {
    errorText = friendly(err);
  }
  render();
}

async function turnOn() {
  busy = "installing";
  errorText = "";
  tab = "download";
  render();
  try {
    status = await startLocalCluster();
    peersDraft = status.peer_hosts ?? peersDraft;
  } catch (err) {
    errorText = friendly(err);
  } finally {
    busy = "idle";
    progress = null;
    render();
  }
}

async function useModel(modelId: string) {
  const tier = status?.tiers.find((item) => item.model_id === modelId);
  if (!tier || !tier.fits_cluster) return;
  pendingModelId = modelId;
  busy = "preparing";
  if (!tier.on_disk) tab = "download";
  errorText = "";
  render();
  try {
    if (peersDraft.trim()) {
      status = await setLlamaPeerHosts(peersDraft);
    }
    status = await activateClusterModel(modelId);
    peersDraft = status.peer_hosts ?? peersDraft;
    pendingModelId = status.selected_model;
    void openAskWindow({ runtime: "doctor" }).catch(() => {});
  } catch (err) {
    errorText = friendly(err);
  } finally {
    busy = "idle";
    progress = null;
    render();
  }
}

async function savePeers() {
  errorText = "";
  try {
    status = await setLlamaPeerHosts(peersDraft);
    peersDraft = status.peer_hosts ?? peersDraft;
  } catch (err) {
    errorText = friendly(err);
  }
  render();
}

async function addPeer() {
  const host = normalizePeer(addDraft);
  if (!host) return;
  const merged = peerTokens(peersDraft);
  if (!merged.includes(host)) merged.push(host);
  peersDraft = merged.join(", ");
  addDraft = "";
  addingPeer = false;
  await savePeers();
}

async function removePeer(host: string) {
  peersDraft = peerTokens(peersDraft)
    .filter((item) => item !== host)
    .join(", ");
  foundPeers = foundPeers.filter((item) => item !== host);
  await savePeers();
}

async function scan(quiet = false) {
  if (!quiet) {
    scanning = true;
    errorText = "";
    render();
  }
  let changed = false;
  try {
    const found = (await scanLlamaPeers()).map(normalizePeer).filter(Boolean);
    foundPeers = found;
    const merged = peerTokens(peersDraft);
    for (const host of found) {
      if (!merged.includes(host)) merged.push(host);
    }
    if (merged.join(", ") !== peerTokens(peersDraft).join(", ")) {
      peersDraft = merged.join(", ");
      status = await setLlamaPeerHosts(peersDraft);
      peersDraft = status.peer_hosts ?? peersDraft;
      changed = true;
    }
  } catch (err) {
    foundPeers = [];
    errorText = friendly(err);
    changed = true;
  } finally {
    scanning = false;
    if (!quiet || changed) render();
  }
}

function bind() {
  document.body.addEventListener("click", (event) => {
    const target = event.target as HTMLElement;
    const tabBtn = target.closest<HTMLButtonElement>("[data-board-tab]");
    if (tabBtn?.dataset.boardTab === "recommend" || tabBtn?.dataset.boardTab === "download" || tabBtn?.dataset.boardTab === "network") {
      tab = tabBtn.dataset.boardTab;
      render();
      return;
    }
    const actionBtn = target.closest<HTMLButtonElement>("[data-board-action]");
    const action = actionBtn?.dataset.boardAction;
    if (!action) return;
    if (action === "model-source" && (actionBtn.dataset.source === "modelscope" || actionBtn.dataset.source === "huggingface")) {
      void pickSource(actionBtn.dataset.source);
    } else if (action === "pick" && actionBtn.dataset.model) {
      pendingModelId = actionBtn.dataset.model;
      render();
    } else if (action === "turn-on") {
      void turnOn();
    } else if (action === "use") {
      void useModel(chosenModel());
    } else if (action === "use-saved" && actionBtn.dataset.model) {
      void useModel(actionBtn.dataset.model);
    } else if (action === "cancel") {
      void cancelClusterInstall();
      busy = "idle";
      progress = null;
      render();
    } else if (action === "scan") {
      void scan();
    } else if (action === "show-add") {
      addingPeer = true;
      render();
      document.querySelector<HTMLInputElement>("#llama-board-peers")?.focus();
    } else if (action === "save-peers") {
      const input = document.querySelector<HTMLInputElement>("#llama-board-peers");
      addDraft = input?.value ?? addDraft;
      void addPeer();
    } else if (action === "remove-peer" && actionBtn.dataset.peer) {
      void removePeer(actionBtn.dataset.peer);
    }
  });
  document.body.addEventListener("input", (event) => {
    const input = event.target as HTMLInputElement;
    if (input.id === "llama-board-peers") addDraft = input.value;
  });
}

async function boot() {
  mountWindowChrome();
  applyStaticI18n();
  bind();
  try {
    status = await getClusterStatus();
    peersDraft = status.peer_hosts ?? "";
  } catch (err) {
    errorText = friendly(err);
  }
  render();
  void scan();
  window.setInterval(() => {
    if (addingPeer || busy !== "idle" || scanning) return;
    void scan(true);
  }, 20000);
  void listen<ClusterInstallProgress>("cluster-install-progress", (ev) => {
    progress = ev.payload;
    if (document.activeElement?.id === "llama-board-peers") return;
    render();
  });
  void listen<ClusterStatus>("cluster-status-changed", (ev) => {
    status = ev.payload;
    if (busy === "idle") progress = null;
    if (!peersDraft.trim()) peersDraft = ev.payload.peer_hosts ?? "";
    if (document.activeElement?.id === "llama-board-peers") return;
    render();
  });
}

void boot();
