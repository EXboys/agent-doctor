import { isPersonalEdition } from "./edition";
import type {
  BrowserMcpTargetAction,
  DoctorReport,
  McpModuleStatus,
  SkillAgentUsage,
  SkillsInventoryReport,
  TeamupsAccountStatus,
  TeamupsCatalogItem,
  TeamupsMallCatalog,
  ResourceRow,
} from "./types";
import type { SkillCategoryId } from "./skill-categories";

export type ResourcesSection = "agents" | "skills" | "tools" | "browser";

export type SkillScope = "local" | "store";

export type SkillFilter = "all" | "issue" | "store" | SkillCategoryId;

export type ToolFilter = "all" | "issue";

export type MallFilter = "all" | "free" | "paid" | "pack" | "skill";

export type UnifiedSkillEntry = {
  key: string;
  name: string;
  description: string;
  category: SkillCategoryId;
  badgeLabel: string;
  sub: string;
  meta: string;
  tone: ResourceRow["tone"];
  issue: boolean;
  skillId?: string;
  needsMount?: boolean;
  canUnmount?: boolean;
  mallItem?: TeamupsCatalogItem;
  storeOnly: boolean;
  iconKind: "skill" | "pack" | "mcp";
  agents?: SkillAgentUsage[];
};

export const MCP_SHOW_UI_KEY = "agent-doctor.mcp.showUi";
export const MCP_USER_DATA_DIR_KEY = "agent-doctor.mcp.userDataDir";
export const MCP_PROFILE_DIRECTORY_KEY = "agent-doctor.mcp.profileDirectory";
export const subtitleEl = document.querySelector<HTMLElement>("#resources-subtitle");
export const sectionTabsEl = document.querySelector<HTMLElement>("#resources-section-tabs")!;
export const mainHeadEl = document.querySelector<HTMLElement>("#resources-main-head")!;
export const skillsPanelEl = document.querySelector<HTMLElement>("#panel-skills")!;
export const skillScopeEl = document.querySelector<HTMLElement>("#resources-skill-scope")!;
export const skillScopeHintEl = document.querySelector<HTMLElement>("#resources-scope-hint")!;
export const panelLeadEl = document.querySelector<HTMLElement>("#resources-panel-lead")!;
export const skillsStickyEl = document.querySelector<HTMLElement>("#resources-skills-sticky")!;
export const skillFiltersEl = document.querySelector<HTMLElement>("#resources-skill-filters")!;
export const agentBarEl = document.querySelector<HTMLElement>("#resources-agent-bar")!;
export const agentFiltersEl = document.querySelector<HTMLElement>("#resources-agent-filters")!;
export const agentHintEl = document.querySelector<HTMLElement>("#resources-agent-hint")!;
export const toolFiltersEl = document.querySelector<HTMLElement>("#resources-tool-filters")!;
export const searchEl = document.querySelector<HTMLInputElement>("#resources-search")!;
export const listEl = document.querySelector<HTMLElement>("#resources-list")!;
export const emptyEl = document.querySelector<HTMLElement>("#resources-empty")!;
export const footnoteEl = document.querySelector<HTMLElement>("#resources-footnote")!;
export const toolsListEl = document.querySelector<HTMLUListElement>("#resources-tools-list")!;
export const toolsEmptyEl = document.querySelector<HTMLElement>("#resources-tools-empty")!;
export const toolsFootnoteEl = document.querySelector<HTMLElement>("#resources-tools-footnote")!;
export const mallAccountEl = document.querySelector<HTMLElement>("#resources-mall-account")!;
export const mallAccountStatusEl = document.querySelector<HTMLElement>("#resources-mall-account-status")!;
export const mallLoginEl = document.querySelector<HTMLButtonElement>("#resources-mall-login")!;
export const mallLogoutEl = document.querySelector<HTMLButtonElement>("#resources-mall-logout")!;
export const agentsListEl = document.querySelector<HTMLUListElement>("#resources-agents-list")!;
export const agentsEmptyEl = document.querySelector<HTMLElement>("#resources-agents-empty")!;
export const mcpBrowserBadgeEl = document.querySelector<HTMLElement>("#mcp-browser-badge")!;
export const mcpChromeEl = document.querySelector<HTMLElement>("#mcp-chrome")!;
export const mcpCdpEl = document.querySelector<HTMLElement>("#mcp-cdp")!;
export const mcpConfiguredEl = document.querySelector<HTMLElement>("#mcp-configured")!;
export const mcpBinaryEl = document.querySelector<HTMLElement>("#mcp-binary")!;
export const mcpShowUiEl = document.querySelector<HTMLInputElement>("#mcp-show-ui")!;
export const mcpUserDataDirEl = document.querySelector<HTMLInputElement>("#mcp-user-data-dir")!;
export const mcpProfileDirectoryEl = document.querySelector<HTMLInputElement>("#mcp-profile-directory")!;
export const mcpProfileSystemEl = document.querySelector<HTMLButtonElement>("#mcp-profile-system")!;
export const mcpProfileIsolatedEl = document.querySelector<HTMLButtonElement>("#mcp-profile-isolated")!;
export const mcpRefreshEl = document.querySelector<HTMLButtonElement>("#mcp-refresh")!;
export const mcpDiagnoseWireEl = document.querySelector<HTMLButtonElement>("#mcp-diagnose-wire")!;
export const mcpTargetsEl = document.querySelector<HTMLUListElement>("#mcp-targets")!;
export const mcpSnippetEl = document.querySelector<HTMLElement>("#mcp-snippet")!;
export const mcpFootnoteEl = document.querySelector<HTMLElement>("#mcp-footnote")!;
export const agentInstallInFlight = new Set<string>();
export const agentOpenInFlight = new Set<string>();
export const agentInstallHint = new Map<string, string>();
export const personalEdition = isPersonalEdition();

export const resourcesState = {
  lastSkillsInventory: null as SkillsInventoryReport | null,
  lastMcpStatus: null as McpModuleStatus | null,
  lastMallCatalog: null as TeamupsMallCatalog | null,
  lastDoctorReport: null as DoctorReport | null,
  lastTeamupsAccount: null as TeamupsAccountStatus | null,
  lastWireActions: null as BrowserMcpTargetAction[] | null,
  skillFilter: "all" as SkillFilter,
  toolFilter: "all" as ToolFilter,
  mallFilter: "all" as MallFilter,
  resourceQuery: "",
  activeSection: "skills" as ResourcesSection,
  mcpConfigureInFlight: false,
  mallActionInFlight: false,
  mallLoginInFlight: false,
  mallLoginTimer: null as number | null,
  skillsStatusMessage: null as string | null,
  highlightSkillKey: null as string | null,
  skillNavActive: "all" as SkillFilter,
  agentFilter: "all" as string,
  skillScrollLock: false,
  skillScrollUnlockTimer: null as number | null,
  skillGroupObserver: null as IntersectionObserver | null,
  cursorOnThisComputer: false,
  refreshInFlight: null as Promise<void> | null,
  lastRefreshAt: 0,
};
