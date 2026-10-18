/** Open files in the side panel, and the folder those files belong to. */

export type OpenFileTab = {
  key: string;
  name: string;
  relative: string;
  root: string;
  external: boolean;
};

export function tabKey(root: string, relative: string): string {
  return `${root}\n${relative}`;
}

/** The folder that contains this file. Empty when the file is at the panel root. */
export function parentFolder(relative: string): string {
  const parts = relative.split("/").filter(Boolean);
  parts.pop();
  return parts.join("/");
}

/** Opening a file again keeps one tab and moves it to the end. */
export function rememberOpenTab(tabs: readonly OpenFileTab[], tab: OpenFileTab): OpenFileTab[] {
  return [...tabs.filter((item) => item.key !== tab.key), tab];
}

/** Closing the open file selects the neighbor. Closing another file keeps the current one. */
export function closeOpenTab(
  tabs: readonly OpenFileTab[],
  key: string,
): { tabs: OpenFileTab[]; next: OpenFileTab | null } {
  const index = tabs.findIndex((item) => item.key === key);
  if (index < 0) return { tabs: [...tabs], next: tabs[tabs.length - 1] ?? null };
  const next = tabs[index + 1] ?? tabs[index - 1] ?? null;
  return { tabs: tabs.filter((item) => item.key !== key), next };
}
