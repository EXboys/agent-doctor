import { fetchRuntimeCatalog } from "./ipc";
import { t } from "./i18n";
import { DOCTOR_RUNTIME_ID } from "./agent-tiles";

export { DOCTOR_RUNTIME_ID };

export type RuntimeCatalogEntry = {
  id: string;
  label: string;
  ask: boolean;
  opens_app: boolean;
  browser_mcp: boolean;
  skill_mount: boolean;
};

let entries: RuntimeCatalogEntry[] = [];

export async function loadRuntimeCatalog(): Promise<void> {
  try {
    entries = await fetchRuntimeCatalog();
  } catch {
    entries = [];
  }
  // The built-in doctor agent has no registry descriptor; it is always an
  // ask-capable choice and can never be installed or opened as an app.
  entries = [
    ...entries,
    {
      id: DOCTOR_RUNTIME_ID,
      label: t("chat.doctorRuntimeName"),
      ask: true,
      opens_app: false,
      browser_mcp: false,
      skill_mount: false,
    },
  ];
}

export function runtimeCatalog(): readonly RuntimeCatalogEntry[] {
  return entries;
}

function entryFor(runtimeId: string): RuntimeCatalogEntry | undefined {
  return entries.find((entry) => entry.id === runtimeId);
}

export function runtimeLabel(runtimeId: string): string {
  return entryFor(runtimeId)?.label ?? runtimeId;
}

export function isKnownRuntimeId(runtimeId: string): boolean {
  return entryFor(runtimeId) !== undefined;
}

export function isAskRuntimeId(runtimeId: string): boolean {
  return entryFor(runtimeId)?.ask === true;
}

export function isDesktopAppRuntimeId(runtimeId: string): boolean {
  return entryFor(runtimeId)?.opens_app === true;
}

export function supportsBrowserMcp(runtimeId: string): boolean {
  return entryFor(runtimeId)?.browser_mcp === true;
}

export function skillMountRuntimeIds(): string[] {
  return entries.filter((entry) => entry.skill_mount).map((entry) => entry.id);
}

export function agentFilterRuntimeIds(): string[] {
  return entries.map((entry) => entry.id);
}
