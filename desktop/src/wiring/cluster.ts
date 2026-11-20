import { listen } from "@tauri-apps/api/event";
import { isTeamEdition } from "../edition";
import { escapeHtml } from "../format";
import { getLocale, t } from "../i18n";
import {
  activateClusterModel,
  cancelClusterInstall,
  connectRemoteCluster,
  getClusterStatus,
  healLocalCluster,
  openAskWindow,
  openClusterBoardWindow,
  openLlamaBoardWindow,
  scanLanClusters,
  scanLlamaPeers,
  setClusterPairingCode,
  setClusterBackend,
  setLlamaPeerHosts,
  startLocalCluster,
  stopCluster,
} from "../ipc";
import type {
  ClusterInstallProgress,
  ClusterModelTier,
  ClusterStatus,
  LanClusterCandidate,
} from "../types";

const GIB = 1024 * 1024 * 1024;
const MIB = 1024 * 1024;

export interface ClusterControllerDeps {
  onProviderChanged: () => Promise<void>;
}

export function createClusterController(deps: ClusterControllerDeps) {
  const cardEl = document.querySelector<HTMLElement>("#cluster-card");

  let status: ClusterStatus | null = null;
  let busyMode: "idle" | "installing" | "starting" | "preparing" | "scanning" | "connecting" = "idle";
  let installProgress: ClusterInstallProgress | null = null;
  let lanCandidates: LanClusterCandidate[] | null = null;
  let pendingModelId: string | null = null;
  let feedback: { tone: "ok" | "error" | "info"; text: string } | null = null;
  let manualHostDraft = "";
  let peerDraft: string | null = null;
  let llamaScanState: "idle" | "scanning" | "done" = "idle";
  let joiningPeers = false;
  let healTimer: number | null = null;
  let llamaFoundCount = 0;

  function formatGb(bytes: number): string {
    if (!bytes || bytes <= 0) return "—";
    const gb = bytes / GIB;
    return gb >= 10 ? String(Math.round(gb)) : gb.toFixed(1);
  }

  function formatMb(bytes: number): string {
    return (bytes / MIB).toFixed(0);
  }

  function tierDisplayName(tier: ClusterModelTier): string {
    if (tier.id === "fast") return t("cluster.tierFast");
    if (tier.id === "balanced") return t("cluster.tierBalanced");
    if (tier.id === "large") return t("cluster.tierLarge");
    return tier.id;
  }

  function friendlyClusterError(err: unknown): string {
    const raw = err instanceof Error ? err.message : String(err ?? "");
    const clean = raw.replace(/^Error:\s*/i, "").trim();
    if (/[\u4e00-\u9fa5]/.test(clean) && getLocale() !== "en") {
      return clean;
    }
    return clean || t("personal.applyFailed");
  }

  function selectedModelId(s: ClusterStatus): string {
    if (pendingModelId && s.tiers.some((tier) => tier.model_id === pendingModelId)) {
      return pendingModelId;
    }
    return s.selected_model;
  }

  function summarySentence(s: ClusterStatus): string {
    const modelDownload = installProgress?.stage === "model";
    if (busyMode === "installing" || modelDownload) {
      if (installProgress?.stage === "extracting") {
        return t("cluster.extracting");
      }
      const done = formatMb(installProgress?.downloaded_bytes ?? 0);
      if (installProgress?.total_bytes && installProgress.total_bytes > 0) {
        const total = formatMb(installProgress.total_bytes);
        return modelDownload
          ? t("cluster.downloadingModel", { done, total })
          : t("cluster.downloadingProgress", { done, total });
      }
      return modelDownload
        ? t("cluster.downloadingModelNoTotal", { done })
        : t("cluster.downloadingNoTotal", { done });
    }
    if (busyMode === "starting") {
      return t("cluster.starting");
    }
    if (busyMode === "preparing") {
      return t("cluster.preparingModel", { count: String(Math.max(1, s.nodes.length)) });
    }
    if (busyMode === "scanning") {
      return t("cluster.scanningLan");
    }

    if (s.backend === "llama_cpp") {
      if (s.reachable) return t("cluster.llamaRunning");
      if (s.runtime_installed) return t("cluster.llamaReady");
      return s.exo_compatible ? t("cluster.llamaChosen") : t("cluster.llamaIdle");
    }

    if (s.reachable) {
      if (
        (joiningPeers || healTimer !== null) &&
        s.backend === "exo" &&
        s.nodes.length < 2 &&
        !s.remote_origin
      ) {
        return t("cluster.joiningPeers");
      }
      const count = String(Math.max(1, s.nodes.length));
      const gb = formatGb(s.usable_memory_bytes);
      const memory = gb === "—" ? "" : t("cluster.memoryNote", { gb });
      if (s.remote_origin) {
        return t("cluster.remoteConnected", { count, memory });
      }
      if (s.nodes.length >= 2) {
        return t("cluster.onlineMulti", { count, memory });
      }
      if (s.namespace.source === "account") {
        return t("cluster.onlineSingleAccount", { memory });
      }
      return t("cluster.onlineSingleCode", {
        memory,
        code: s.namespace.pairing_code || "—",
      });
    }

    if (s.host_support === "supported") {
      return s.runtime_installed
        ? t("cluster.macIdleInstalled")
        : t("cluster.macIdleUninstalled");
    }
    if (s.host_support === "mac_os_too_old") {
      return t("cluster.macOSOld");
    }
    return t("cluster.consumerIdle");
  }

  function shouldKeepJoining(s: ClusterStatus): boolean {
    return (
      s.backend === "exo" &&
      s.reachable &&
      !s.remote_origin &&
      s.nodes.length < 2 &&
      s.daemon_owner !== "external_process" &&
      s.host_support === "supported"
    );
  }

  function scheduleHeal(s: ClusterStatus) {
    if (!shouldKeepJoining(s)) {
      if (healTimer !== null) {
        window.clearInterval(healTimer);
        healTimer = null;
      }
      return;
    }
    if (healTimer !== null) return;
    void runHeal();
    healTimer = window.setInterval(() => {
      void runHeal();
    }, 20000);
  }

  async function runHeal() {
    if (joiningPeers) return;
    const current = status;
    if (!current || !shouldKeepJoining(current)) return;
    joiningPeers = true;
    render();
    try {
      status = await healLocalCluster();
    } catch {
      // Keep the last status. The next pass tries again.
    } finally {
      joiningPeers = false;
      render();
      if (status) scheduleHeal(status);
    }
  }

  function maybeDiscoverLlama(s: ClusterStatus) {
    if (s.backend !== "llama_cpp" || llamaScanState !== "idle") return;
    llamaScanState = "scanning";
    render();
    void scanLlamaPeers()
      .then(async (found) => {
        llamaFoundCount = found.length;
        const current = (peerDraft ?? s.peer_hosts ?? "").trim();
        if (found.length > 0 && current === "") {
          peerDraft = found.map((host) => host.replace(/:50052$/, "")).join(", ");
          try {
            status = await setLlamaPeerHosts(peerDraft);
          } catch {
            // Keep the filled addresses on screen even if saving them failed.
          }
        }
      })
      .catch(() => {
        llamaFoundCount = 0;
      })
      .finally(() => {
        llamaScanState = "done";
        render();
      });
  }

  function backendSwitchHtml(s: ClusterStatus, isBusy: boolean): string {
    if (s.offered_backends.length < 2) return "";
    const pills = s.offered_backends
      .filter((kind) => kind === "exo" || kind === "llama_cpp")
      .map((kind) => {
        const selected = kind === s.backend;
        const label = kind === "exo" ? t("cluster.backendExo") : t("cluster.backendLlama");
        return `<button type="button" class="cluster-tier-pill${
          selected ? " is-selected" : ""
        }" role="radio" aria-checked="${selected ? "true" : "false"}" data-cluster-action="pick-backend" data-backend="${kind}" ${
          isBusy ? "disabled" : ""
        }>${escapeHtml(label)}</button>`;
      })
      .join("");
    return `
      <div class="cluster-tier-block">
        <span class="cluster-tier-label">${escapeHtml(t("cluster.backendLabel"))}</span>
        <div class="cluster-tier-pills" role="radiogroup" aria-label="${escapeHtml(
          t("cluster.backendLabel"),
        )}">${pills}</div>
      </div>`;
  }

  function render() {
    if (!cardEl) return;
    if (isTeamEdition()) {
      cardEl.hidden = true;
      cardEl.innerHTML = "";
      return;
    }
    if (!status) {
      cardEl.hidden = true;
      return;
    }

    cardEl.hidden = false;
    const s = status;
    const isBusy = busyMode !== "idle";
    const chosenModel = selectedModelId(s);
    const modelAlreadyWired = s.provider_active && s.active_models.includes(chosenModel);
    const badgeHtml = s.provider_active && s.reachable
      ? `<span class="cluster-badge is-active">${escapeHtml(t("cluster.badgeActive"))}</span>`
      : `<span class="cluster-badge">${escapeHtml(t("cluster.badgeFree"))}</span>`;

    const mismatchHtml =
      s.backend !== "llama_cpp" && s.reachable && s.incompatible_peer_count > 0
        ? `<p class="cluster-warning">${escapeHtml(
            t("cluster.versionMismatch", { count: String(s.incompatible_peer_count) }),
          )}</p>`
        : "";

    let controlsHtml = "";

    if (s.backend === "llama_cpp") {
      const stop =
        s.daemon_owner === "managed_by_us" || s.reachable
          ? `<button type="button" class="btn-ghost btn-compact" data-cluster-action="stop-cluster" ${
              isBusy ? "disabled" : ""
            }>${escapeHtml(t("cluster.btnStop"))}</button>`
          : "";
      const found =
        llamaFoundCount > 0
          ? `<p class="cluster-next">${escapeHtml(
              t("cluster.llamaFoundShort", { count: String(llamaFoundCount) }),
            )}</p>`
          : "";
      controlsHtml = `
        <div class="cluster-actions">
          <button type="button" class="btn-primary btn-compact" data-cluster-action="open-llama-board" ${
            isBusy ? "disabled" : ""
          }>${escapeHtml(t("cluster.btnOpenBoard"))}</button>
          ${stop}
        </div>
        ${found}`;
    } else if (busyMode === "installing" || busyMode === "preparing") {
      controlsHtml = `
        <div class="cluster-actions">
          <button type="button" class="btn-secondary btn-compact" data-cluster-action="cancel-install">
            ${escapeHtml(t("cluster.btnCancelDownload"))}
          </button>
        </div>`;
    } else if (s.reachable) {
      const applyLabel = modelAlreadyWired
        ? ""
        : s.provider_active
          ? t("cluster.btnSwitchModel")
          : t("cluster.btnApplyTier");

      const stopLabel = s.remote_origin ? t("cluster.btnDisconnect") : t("cluster.btnStop");

      const tierPills = s.tiers
        .map((tier) => {
          const selected = tier.model_id === chosenModel;
          const locked = !tier.fits_cluster;
          const minGb = String(Math.round(tier.min_usable_bytes / GIB));
          const hint = locked ? t("cluster.tierNeedsGb", { gb: minGb }) : "";
          return `<button type="button" class="cluster-tier-pill${selected ? " is-selected" : ""}${
            locked ? " is-locked" : ""
          }" role="radio" aria-checked="${selected ? "true" : "false"}" data-cluster-action="pick-tier" data-model="${escapeHtml(
            tier.model_id,
          )}" ${locked || isBusy ? "disabled" : ""}>
            <span class="cluster-tier-pill-name">${escapeHtml(tierDisplayName(tier))}</span>
            ${hint ? `<span class="cluster-tier-pill-hint">${escapeHtml(hint)}</span>` : ""}
          </button>`;
        })
        .join("");

      controlsHtml = `
        <div class="cluster-tier-block">
          <span class="cluster-tier-label">${escapeHtml(t("cluster.tierLabel"))}</span>
          <div class="cluster-tier-pills" role="radiogroup" aria-label="${escapeHtml(
            t("cluster.tierLabel"),
          )}">${tierPills}</div>
        </div>
        <p class="cluster-next">${escapeHtml(
          modelAlreadyWired ? t("cluster.chatUsesTier") : t("cluster.pickThenChat"),
        )}</p>
        <div class="cluster-actions">
          ${
            applyLabel
              ? `<button type="button" class="btn-primary btn-compact" data-cluster-action="activate-model" ${
                  isBusy ? "disabled" : ""
                }>${escapeHtml(applyLabel)}</button>`
              : ""
          }
          ${
            s.dashboard_url
              ? `<button type="button" class="cluster-board-link" data-cluster-action="open-dashboard" data-url="${escapeHtml(
                  s.dashboard_url,
                )}">${escapeHtml(t("cluster.btnDashboard"))}</button>`
              : ""
          }
          ${
            s.daemon_owner === "managed_by_us" || s.remote_origin
              ? `<button type="button" class="btn-ghost btn-compact" data-cluster-action="stop-cluster" ${
                  isBusy ? "disabled" : ""
                }>${escapeHtml(stopLabel)}</button>`
              : ""
          }
        </div>`;
    } else if (s.host_support === "supported") {
      const startLabel = s.runtime_installed
        ? t("cluster.btnStart")
        : t("cluster.btnDownloadStart");
      controlsHtml = `
        <div class="cluster-actions">
          <button type="button" class="btn-primary btn-compact" data-cluster-action="start-local" ${
            isBusy ? "disabled" : ""
          }>${escapeHtml(startLabel)}</button>
        </div>`;
    } else {
      const candidatesHtml =
        lanCandidates && lanCandidates.length > 0
          ? `<ul class="cluster-lan-list">${lanCandidates
              .map((cand) => {
                const devices = cand.device_labels.join(" · ") || cand.host;
                const modelAttr = cand.active_models[0]
                  ? ` data-model="${escapeHtml(cand.active_models[0])}"`
                  : "";
                return `<li class="cluster-lan-item">
                  <span>${escapeHtml(
                    t("cluster.scanFoundItem", {
                      count: String(cand.node_count),
                      gb: formatGb(cand.usable_memory_bytes),
                      devices,
                    }),
                  )}</span>
                  <button type="button" class="btn-primary btn-compact" data-cluster-action="connect-candidate" data-origin="${escapeHtml(
                    cand.origin,
                  )}"${modelAttr} ${isBusy ? "disabled" : ""}>${escapeHtml(
                    t("cluster.btnConnectCandidate"),
                  )}</button>
                </li>`;
              })
              .join("")}</ul>`
          : "";

      const showManualRow = lanCandidates !== null && lanCandidates.length === 0;
      const manualHtml = showManualRow
        ? `<p class="cluster-empty-note">${escapeHtml(t("cluster.scanEmpty"))}</p>
           <div class="cluster-manual-row">
             <input id="cluster-manual-ip" type="text" class="cluster-input" value="${escapeHtml(
               manualHostDraft,
             )}" placeholder="${escapeHtml(t("cluster.manualIpPlaceholder"))}" ${
               isBusy ? "disabled" : ""
             } />
             <button type="button" class="btn-secondary btn-compact" data-cluster-action="connect-manual" ${
               isBusy ? "disabled" : ""
             }>${escapeHtml(t("cluster.btnConnectIp"))}</button>
           </div>`
        : "";

      controlsHtml = `
        <div class="cluster-actions">
          <button type="button" class="btn-primary btn-compact" data-cluster-action="scan-lan" ${
            isBusy ? "disabled" : ""
          }>${escapeHtml(t("cluster.btnScanLan"))}</button>
        </div>
        ${candidatesHtml}
        ${manualHtml}`;
    }

    const feedbackHtml = feedback
      ? `<p class="cluster-feedback is-${feedback.tone}">${escapeHtml(feedback.text)}</p>`
      : "";

    let detailsHtml = "";
    if (s.backend !== "llama_cpp" && (s.host_support === "supported" || s.reachable)) {
      const sep = getLocale() === "en" ? ", " : "、";
      const deviceNames = s.nodes.map((n) => n.label).join(sep);
      const customCodeVal =
        s.namespace.source === "custom_code" ? s.namespace.pairing_code ?? "" : "";
      detailsHtml = `
        <details class="cluster-details">
          <summary>${escapeHtml(t("cluster.detailsSummary"))}</summary>
          <div class="cluster-details-body">
            ${
              deviceNames
                ? `<p>${escapeHtml(t("cluster.detailsDevices", { devices: deviceNames }))}</p>`
                : ""
            }
            <p>${escapeHtml(t("cluster.detailsModel", { model: chosenModel }))}</p>
            ${
              s.host_support === "supported"
                ? `<div class="cluster-code-box">
                     <label for="cluster-pairing-code">${escapeHtml(
                       t("cluster.detailsCodeLabel"),
                     )}</label>
                     <div class="cluster-manual-row">
                       <input id="cluster-pairing-code" type="text" class="cluster-input" value="${escapeHtml(
                         customCodeVal,
                       )}" placeholder="${escapeHtml(t("cluster.detailsCodePlaceholder"))}" />
                       <button type="button" class="btn-secondary btn-compact" data-cluster-action="save-code" ${
                         isBusy ? "disabled" : ""
                       }>${escapeHtml(t("cluster.detailsCodeSave"))}</button>
                     </div>
                   </div>
                   <p class="cluster-net-hint">${escapeHtml(t("cluster.detailsLocalNetHint"))}</p>`
                : ""
            }
          </div>
        </details>`;
    }

    cardEl.classList.toggle("is-active", s.provider_active && s.reachable);
    cardEl.innerHTML = `
      <div class="cluster-card-head">
        <div class="cluster-title">
          <span class="cluster-mark" aria-hidden="true">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <rect x="3" y="4" width="7" height="7" rx="1.5"></rect>
              <rect x="14" y="4" width="7" height="7" rx="1.5"></rect>
              <rect x="8.5" y="13.5" width="7" height="7" rx="1.5"></rect>
            </svg>
          </span>
          <h3>${escapeHtml(t("cluster.title"))}</h3>
        </div>
        ${badgeHtml}
      </div>
      ${backendSwitchHtml(s, isBusy)}
      <p class="cluster-summary">${escapeHtml(summarySentence(s))}</p>
      ${mismatchHtml}
      ${controlsHtml}
      ${feedbackHtml}
      ${detailsHtml}
    `;
  }

  async function load(): Promise<void> {
    if (isTeamEdition() || !cardEl) return;
    try {
      status = await getClusterStatus();
      render();
      if (status?.backend === "llama_cpp") maybeDiscoverLlama(status);
      if (status) scheduleHeal(status);
    } catch {
      // Keep UI quiet on transient probe failure.
    }
  }

  async function handleStartLocal() {
    if (!status) return;
    feedback = null;
    installProgress = null;
    busyMode = status.runtime_installed ? "starting" : "installing";
    render();
    try {
      if (status.backend === "llama_cpp") {
        const peers = peerDraft ?? status.peer_hosts ?? "";
        status = await setLlamaPeerHosts(peers);
      }
      status = await startLocalCluster();
      feedback = { tone: "ok", text: t("cluster.resultStarted") };
      scheduleHeal(status);
    } catch (err) {
      feedback = { tone: "error", text: friendlyClusterError(err) };
    } finally {
      busyMode = "idle";
      installProgress = null;
      render();
    }
  }

  async function handleStop() {
    feedback = null;
    busyMode = "starting";
    render();
    try {
      status = await stopCluster();
      feedback = { tone: "info", text: t("cluster.resultStopped") };
      await deps.onProviderChanged();
    } catch (err) {
      feedback = { tone: "error", text: friendlyClusterError(err) };
    } finally {
      busyMode = "idle";
      render();
    }
  }

  async function handleActivateModel() {
    if (!status) return;
    const model = selectedModelId(status);
    feedback = null;
    installProgress = null;
    busyMode = "preparing";
    render();
    try {
      if (status.backend === "llama_cpp") {
        const peers =
          cardEl?.querySelector<HTMLInputElement>("#cluster-llama-peers")?.value ??
          peerDraft ??
          status.peer_hosts ??
          "";
        peerDraft = peers;
        status = await setLlamaPeerHosts(peers);
      }
      status = await activateClusterModel(model);
      pendingModelId = status.selected_model;
      feedback = { tone: "ok", text: t("cluster.resultActivated") };
      await deps.onProviderChanged();
      void openAskWindow({ runtime: "doctor" }).catch(() => {});
    } catch (err) {
      feedback = { tone: "error", text: friendlyClusterError(err) };
    } finally {
      busyMode = "idle";
      render();
    }
  }

  async function handleScanLan() {
    feedback = null;
    busyMode = "scanning";
    render();
    try {
      lanCandidates = await scanLanClusters();
    } catch (err) {
      lanCandidates = [];
      feedback = { tone: "error", text: friendlyClusterError(err) };
    } finally {
      busyMode = "idle";
      render();
    }
  }

  async function handleConnectRemote(hostOrUrl: string, modelId?: string | null) {
    feedback = null;
    busyMode = "connecting";
    render();
    try {
      status = await connectRemoteCluster(hostOrUrl, modelId ?? null);
      feedback = { tone: "ok", text: t("cluster.resultConnected") };
      await deps.onProviderChanged();
    } catch (err) {
      feedback = { tone: "error", text: friendlyClusterError(err) };
    } finally {
      busyMode = "idle";
      render();
    }
  }

  async function handleSaveCode(codeRaw: string) {
    feedback = null;
    try {
      status = await setClusterPairingCode(codeRaw.trim() || null);
      feedback = { tone: "ok", text: t("cluster.resultCodeSaved") };
    } catch (err) {
      feedback = { tone: "error", text: friendlyClusterError(err) };
    }
    render();
  }

  async function handlePickBackend(backend: "exo" | "llama_cpp") {
    if (!status || status.backend === backend) return;
    feedback = null;
    try {
      status = await setClusterBackend(backend);
      pendingModelId = null;
      if (backend === "llama_cpp") {
        llamaScanState = "idle";
        llamaFoundCount = 0;
      }
    } catch (err) {
      feedback = { tone: "error", text: friendlyClusterError(err) };
    }
    render();
    if (status?.backend === "llama_cpp") maybeDiscoverLlama(status);
  }

  function bindEvents() {
    if (!cardEl || isTeamEdition()) return;

    cardEl.addEventListener("change", (event) => {
      const target = event.target as HTMLElement;
      if (target instanceof HTMLSelectElement && target.id === "cluster-tier-select") {
        pendingModelId = target.value;
        render();
      }
    });

    cardEl.addEventListener("input", (event) => {
      const target = event.target as HTMLElement;
      if (target instanceof HTMLInputElement && target.id === "cluster-manual-ip") {
        manualHostDraft = target.value;
      }
      if (target instanceof HTMLInputElement && target.id === "cluster-llama-peers") {
        peerDraft = target.value;
      }
    });

    cardEl.addEventListener("click", (event) => {
      const btn = (event.target as HTMLElement).closest<HTMLButtonElement>(
        "[data-cluster-action]",
      );
      const action = btn?.dataset.clusterAction;
      if (!btn || !action) return;

      if (action === "start-local") {
        void handleStartLocal();
      } else if (action === "cancel-install") {
        void cancelClusterInstall();
      } else if (action === "stop-cluster") {
        void handleStop();
      } else if (action === "pick-tier") {
        const model = btn.dataset.model;
        if (model) {
          pendingModelId = model;
          render();
        }
      } else if (action === "activate-model") {
        void handleActivateModel();
      } else if (action === "open-llama-board") {
        void openLlamaBoardWindow();
      } else if (action === "open-dashboard") {
        const url = btn.dataset.url;
        if (url) void openClusterBoardWindow({ url });
      } else if (action === "scan-lan") {
        void handleScanLan();
      } else if (action === "connect-candidate") {
        const origin = btn.dataset.origin;
        const model = btn.dataset.model;
        if (origin) void handleConnectRemote(origin, model ?? null);
      } else if (action === "connect-manual") {
        const input = cardEl.querySelector<HTMLInputElement>("#cluster-manual-ip");
        void handleConnectRemote(input?.value ?? manualHostDraft, null);
      } else if (action === "save-code") {
        const input = cardEl.querySelector<HTMLInputElement>("#cluster-pairing-code");
        void handleSaveCode(input?.value ?? "");
      } else if (action === "pick-backend") {
        const backend = btn.dataset.backend;
        if (backend === "exo" || backend === "llama_cpp") {
          void handlePickBackend(backend);
        }
      }
    });

    void listen<ClusterInstallProgress>("cluster-install-progress", (ev) => {
      installProgress = ev.payload;
      render();
    });

    void listen<ClusterStatus>("cluster-status-changed", (ev) => {
      status = ev.payload;
      if (busyMode === "idle") installProgress = null;
      render();
      if (!joiningPeers) scheduleHeal(status);
    });

    void listen("personal-provider-changed", () => {
      void load();
    });
  }

  return {
    load,
    render,
    bindEvents,
  };
}
