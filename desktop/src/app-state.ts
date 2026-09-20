import type {
  DoctorReport,
  HermesSettings,
  McpModuleStatus,
  ModeStatus,
  PersonalProvidersDocument,
  ProfilesDocument,
  RepairPreviewResponse,
  RepairStatusFilter,
  SkillsInventoryReport,
  WorkspacesDocument,
} from "./types";

/** Shared mutable app state for main-window UI controllers (avoids circular imports). */
export const appState = {
  lastReport: null as DoctorReport | null,
  lastProfiles: null as ProfilesDocument | null,
  lastWorkspaces: null as WorkspacesDocument | null,
  hermesModel: null as HermesSettings | null,
  activeRuntimeId: null as string | null,
  selectedPresetName: "",
  selectedWorkspaceName: "",
  lastModeStatus: null as ModeStatus | null,
  lastSkillsInventory: null as SkillsInventoryReport | null,
  lastMcpStatus: null as McpModuleStatus | null,
  personalProvidersDoc: null as PersonalProvidersDocument | null,
  workspaceBusy: false,
  presetMenuOpen: false,
  agentsWsPickerOpen: false,
};

export const repairPreviewByRuntime = new Map<string, RepairPreviewResponse>();
export const repairFilterByRuntime = new Map<string, RepairStatusFilter>();
export const repairConfirmRuntimeIds = new Set<string>();
