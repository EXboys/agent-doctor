/** Lines a turn added or removed, so opening that file can show them again. */

export type FileChangePreview = {
  additions: number;
  deletions: number;
  addedLines: string[];
  deletedLines: string[];
  lineStart?: number;
  lineEnd?: number;
};

const byPath = new Map<string, Map<string, FileChangePreview>>();

export function resetFileChanges(): void {
  byPath.clear();
}

function norm(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "");
}

export function hasFileChange(change: FileChangePreview | undefined): boolean {
  if (!change) return false;
  return Boolean(
    change.additions || change.deletions || change.addedLines?.length || change.deletedLines?.length,
  );
}

/** One tool keeps one record. A later result replaces the start, it does not add again. */
export function rememberFileChange(path: string, toolId: string, change: FileChangePreview): void {
  const key = norm(path);
  if (!key || !hasFileChange(change)) return;
  const bucket = byPath.get(key) ?? new Map();
  bucket.set(toolId || key, {
    additions: change.additions,
    deletions: change.deletions,
    addedLines: [...(change.addedLines ?? [])],
    deletedLines: [...(change.deletedLines ?? [])],
    lineStart: change.lineStart,
    lineEnd: change.lineEnd,
  });
  byPath.set(key, bucket);
}

function merged(bucket: Map<string, FileChangePreview>): FileChangePreview {
  const out: FileChangePreview = {
    additions: 0,
    deletions: 0,
    addedLines: [],
    deletedLines: [],
  };
  for (const change of bucket.values()) {
    out.additions += change.additions;
    out.deletions += change.deletions;
    out.addedLines.push(...change.addedLines);
    out.deletedLines.push(...change.deletedLines);
    if (change.lineStart) out.lineStart = change.lineStart;
    if (change.lineEnd) out.lineEnd = change.lineEnd;
  }
  out.addedLines = out.addedLines.slice(0, 200);
  out.deletedLines = out.deletedLines.slice(0, 200);
  return out;
}

/** `src/a.css` matches a step that stored `/proj/src/a.css`. */
export function fileChangeFor(relative: string): FileChangePreview | undefined {
  const rel = norm(relative).replace(/^\/+/, "");
  if (!rel) return undefined;
  const direct = byPath.get(rel);
  if (direct) return merged(direct);
  let hit: Map<string, FileChangePreview> | undefined;
  for (const [path, bucket] of byPath) {
    if (path.endsWith(`/${rel}`)) hit = bucket;
  }
  return hit ? merged(hit) : undefined;
}
