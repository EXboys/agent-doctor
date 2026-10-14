export type PageDropPosition = "before" | "after";

export function reorderPagePaths(
  paths: string[],
  from: string,
  to: string,
  position: PageDropPosition,
): string[] {
  if (!from || from === to || !paths.includes(from) || !paths.includes(to)) return paths;
  const next = paths.filter((path) => path !== from);
  let index = next.indexOf(to);
  if (index < 0) return paths;
  if (position === "after") index += 1;
  next.splice(index, 0, from);
  return next;
}
