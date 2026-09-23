import { invoke } from "@tauri-apps/api/core";
import { withErrorDetail } from "../friendly-error";
import { t } from "../i18n";
import type {
  EngineRegisterStatus,
  EvotownStatus,
  OnboardingReport,
  RegisterReport,
  SyncReport,
} from "../types";

const evotownSectionEl = document.querySelector<HTMLElement>("#evotown-section")!;
const evotownStatusEl = document.querySelector<HTMLElement>("#evotown-status")!;
const evotownBadgeEl = document.querySelector<HTMLElement>("#evotown-badge");
const evotownConnectedEl = document.querySelector<HTMLElement>("#evotown-connected")!;
const evotownConnectedUrlEl = document.querySelector<HTMLElement>("#evotown-connected-url")!;
const evotownConnectedMetaEl = document.querySelector<HTMLElement>("#evotown-connected-meta")!;
const evotownFormEl = document.querySelector<HTMLFormElement>("#evotown-form")!;
const evotownUrlEl = document.querySelector<HTMLInputElement>("#evotown-url")!;
const evotownKeyEl = document.querySelector<HTMLInputElement>("#evotown-key")!;
const evotownConnectEl = document.querySelector<HTMLButtonElement>("#evotown-connect")!;
const evotownResyncEl = document.querySelector<HTMLButtonElement>("#evotown-resync")!;
const evotownHintEl = document.querySelector<HTMLElement>("#evotown-hint")!;
const evotownEngineEl = document.querySelector<HTMLElement>("#evotown-engine")!;
const evotownEngineBadgeEl = document.querySelector<HTMLElement>("#evotown-engine-badge")!;
const evotownEngineStatusEl = document.querySelector<HTMLElement>("#evotown-engine-status")!;
const evotownEngineFormEl = document.querySelector<HTMLFormElement>("#evotown-engine-form")!;
const evotownBootstrapEl = document.querySelector<HTMLInputElement>("#evotown-bootstrap")!;
const evotownEngineIdEl = document.querySelector<HTMLInputElement>("#evotown-engine-id")!;
const evotownEngineRotateEl = document.querySelector<HTMLInputElement>("#evotown-engine-rotate")!;
const evotownEngineRegisterEl =
  document.querySelector<HTMLButtonElement>("#evotown-engine-register")!;
const evotownEngineHintEl = document.querySelector<HTMLElement>("#evotown-engine-hint")!;

export type EvotownDeps = {
  refresh: () => Promise<void>;
  loadSkillsInventory: () => Promise<void>;
  hideSkillsInventory: () => void;
  loadModeStatus: () => Promise<void>;
};

export type EvotownApi = ReturnType<typeof createEvotownController>;

export function createEvotownController(deps: EvotownDeps) {
  let lastEvotownStatus: EvotownStatus | null = null;

  async function loadEngineRegisterStatus() {
    try {
      const status = await invoke<EngineRegisterStatus>("get_engine_register_status_command");
      if (status.registered && status.engine_id) {
        evotownEngineStatusEl.textContent = t("evotown.engineReady", { id: status.engine_id });
        evotownEngineBadgeEl.hidden = false;
        evotownEngineBadgeEl.className = "badge ok";
        evotownEngineBadgeEl.textContent = t("evotown.engineBadgeOk");
        if (!evotownEngineIdEl.value.trim()) {
          evotownEngineIdEl.value = status.engine_id;
        }
      } else {
        evotownEngineStatusEl.textContent = t("evotown.engineMissing");
        evotownEngineBadgeEl.hidden = false;
        evotownEngineBadgeEl.className = "badge muted";
        evotownEngineBadgeEl.textContent = t("evotown.engineBadgeMissing");
      }
    } catch (error) {
      evotownEngineStatusEl.textContent = t("evotown.engineRegisterFailed", {
        error: String(error),
      });
      evotownEngineBadgeEl.hidden = true;
    }
  }

  function renderEvotownStatus(status: EvotownStatus, opts?: { refreshSkills?: boolean }) {
    lastEvotownStatus = status;
    const connected = status.configured && Boolean(status.base_url);
    evotownSectionEl.classList.toggle("is-connected", connected);
    evotownConnectedEl.hidden = !connected;
    evotownStatusEl.hidden = connected;

    if (evotownBadgeEl) {
      if (connected) {
        evotownBadgeEl.hidden = false;
        evotownBadgeEl.className = "badge ok";
        evotownBadgeEl.textContent = t("evotown.connectedBadge");
      } else {
        evotownBadgeEl.hidden = true;
        evotownBadgeEl.textContent = "";
      }
    }

    if (connected && status.base_url) {
      evotownStatusEl.textContent = "";
      evotownConnectedUrlEl.textContent = status.base_url;
      evotownConnectedMetaEl.textContent = t("evotown.meta", {
        runtime: status.runtime_target ?? "openclaw",
        bundle: status.bundle_id ?? "default-agent-skills",
      });
      evotownConnectedMetaEl.title = status.api_key_hint ?? "";
      evotownUrlEl.value = status.base_url;
      evotownResyncEl.hidden = false;
      evotownEngineEl.hidden = false;
      if (opts?.refreshSkills !== false) {
        void loadEngineRegisterStatus();
        void deps.loadSkillsInventory();
      }
    } else {
      evotownStatusEl.textContent = t("evotown.notConfigured");
      evotownConnectedMetaEl.textContent = "";
      evotownConnectedMetaEl.title = "";
      evotownResyncEl.hidden = true;
      evotownEngineEl.hidden = true;
      evotownEngineHintEl.textContent = "";
      deps.hideSkillsInventory();
    }
  }

  async function loadEvotownStatus() {
    try {
      const status = await invoke<EvotownStatus>("get_evotown_status_command");
      renderEvotownStatus(status);
    } catch (error) {
      evotownStatusEl.textContent = withErrorDetail(t("evotown.connectFailed"), error);
    }
  }

  async function runEvotownOnboarding() {
    const url = evotownUrlEl.value.trim();
    const key = evotownKeyEl.value.trim();
    if (!url || !key) {
      evotownHintEl.textContent = t("evotown.connectFailed", {
        error: "URL and API key are required",
      });
      return;
    }

    evotownConnectEl.disabled = true;
    evotownResyncEl.disabled = true;
    evotownHintEl.textContent = t("evotown.connecting");
    try {
      const report = await invoke<OnboardingReport>("run_evotown_onboarding_command", {
        url,
        key,
        syncSkills: true,
        pullPolicies: true,
      });
      evotownKeyEl.value = "";
      await loadEvotownStatus();
      await deps.loadModeStatus();
      await deps.refresh();
      evotownHintEl.textContent = t("evotown.connectOk", {
        installed: String(report.sync?.installed ?? 0),
        policies: String(report.policy?.policy_count ?? 0),
      });
    } catch (error) {
      evotownHintEl.textContent = withErrorDetail(t("evotown.connectFailed"), error);
    } finally {
      evotownConnectEl.disabled = false;
      evotownResyncEl.disabled = false;
    }
  }

  async function runEngineRegister() {
    const bootstrap = evotownBootstrapEl.value.trim();
    if (!bootstrap) {
      evotownEngineHintEl.textContent = t("evotown.engineTokenRequired");
      return;
    }

    evotownEngineRegisterEl.disabled = true;
    evotownEngineHintEl.textContent = t("evotown.engineRegistering");
    try {
      const report = await invoke<RegisterReport>("run_engine_register_command", {
        bootstrapToken: bootstrap,
        engineId: evotownEngineIdEl.value.trim() || null,
        rotate: evotownEngineRotateEl.checked,
      });
      evotownBootstrapEl.value = "";
      evotownEngineRotateEl.checked = false;
      await loadEngineRegisterStatus();
      evotownEngineHintEl.textContent = t("evotown.engineRegisterOk", { id: report.engine_id });
    } catch (error) {
      evotownEngineHintEl.textContent = t("evotown.engineRegisterFailed", {
        error: String(error),
      });
    } finally {
      evotownEngineRegisterEl.disabled = false;
    }
  }

  async function resyncEvotownSkills() {
    evotownResyncEl.disabled = true;
    evotownHintEl.textContent = t("evotown.resyncRunning");
    try {
      const report = await invoke<SyncReport>("run_sync_command");
      evotownHintEl.textContent = t("evotown.resyncOk", {
        installed: String(report.installed),
        skipped: String(report.skipped),
        failed: String(report.failed),
      });
      await deps.loadSkillsInventory();
    } catch (error) {
      evotownHintEl.textContent = withErrorDetail(t("evotown.resyncFailed"), error);
    } finally {
      evotownResyncEl.disabled = false;
    }
  }

  function bindEvents() {
    evotownFormEl.addEventListener("submit", (event) => {
      event.preventDefault();
      void runEvotownOnboarding();
    });

    evotownResyncEl.addEventListener("click", () => {
      void resyncEvotownSkills();
    });

    evotownEngineFormEl.addEventListener("submit", (event) => {
      event.preventDefault();
      void runEngineRegister();
    });
  }

  return {
    loadEvotownStatus,
    renderEvotownStatus,
    getLastStatus: () => lastEvotownStatus,
    bindEvents,
  };
}
