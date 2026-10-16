import type {
  DoctorReport,
  HermesSettings,
  McpModuleStatus,
  ModeStatus,
  NativeAccountRow,
  PersonalProvidersDocument,
  ProfilesDocument,
  RepairPreviewResponse,
  RepairStatusFilter,
  RuntimeVersionStatus,
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
  nativeAccounts: [] as NativeAccountRow[],
  runtimeVersions: new Map<string, RuntimeVersionStatus>(),
  workspaceBusy: false,
  presetMenuOpen: false,
  agentsWsPickerOpen: false,
};

export const repairPreviewByRuntime = new Map<string, RepairPreviewResponse>();
export const repairFilterByRuntime = new Map<string, RepairStatusFilter>();
export const repairConfirmRuntimeIds = new Set<string>();

/** A status read that did not probe must not hide a row the last probe showed. */
export function applyNativeAccounts(rows: NativeAccountRow[], probed: boolean): void {
  if (probed) {
    appState.nativeAccounts = rows;
    return;
  }
  const previous = new Map(appState.nativeAccounts.map((row) => [row.runtime_id, row]));
  appState.nativeAccounts = rows.map((row) => {
    if (row.using_subscription) return { ...row, visible: true };
    const earlier = previous.get(row.runtime_id);
    if (!earlier?.reachable) return row;
    return { ...row, reachable: true, visible: earlier.visible };
  });
}
