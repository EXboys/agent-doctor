import type { ChannelReplyImage } from "../ipc";

export const MAX_CHANNEL_REPLY_IMAGES = 3;
const MAX_IMAGE_CANDIDATES = 6;
const IMAGE_EXT_RE = /\.(?:png|jpe?g|gif|webp)$/i;
const REMOTE_OR_DATA_RE = /^(?:https?:|data:|blob:|mailto:|javascript:)/i;

function cleanCandidatePath(raw: string): string {
  let value = raw.trim();
  if (!value) return "";
  if (value.startsWith("<") && value.endsWith(">")) {
    value = value.slice(1, -1).trim();
  }
  // Strip optional markdown link title: `path.png "Preview"` or `path.png 'Preview'`.
  const titleMatch = value.match(/^(.+?\.(?:png|jpe?g|gif|webp))\s+["'][^"']*["']$/i);
  if (titleMatch) value = titleMatch[1].trim();
  // Strip wrapping quotes or backticks.
  value = value.replace(/^[`"'“”‘’]+|[`"'“”‘’]+$/g, "").trim();
  // Strip trailing Chinese / English sentence punctuation glued to an inline path.
  value = value.replace(/[，。；：！？、）》】\].,;:!?)]+$/g, "").trim();
  if (!value || REMOTE_OR_DATA_RE.test(value)) return "";
  // Drop query/hash suffixes on local paths (`chart.png?v=1`) before checking extension.
  const withoutQuery = value.replace(/[?#][^/\\]*$/, "");
  if (!IMAGE_EXT_RE.test(withoutQuery)) return "";
  return withoutQuery;
}

/**
 * Local picture paths referenced in a channel reply (Markdown `![alt](...)`,
 * links, inline code, standalone path lines, inline prose paths, and newly
 * written image files from this turn's tool steps).
 */
export function extractChannelReplyImagePaths(
  text: string,
  files: { path: string; kind: "change" | "output" }[] = [],
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  const push = (raw: string) => {
    if (out.length >= MAX_IMAGE_CANDIDATES) return;
    const cleaned = cleanCandidatePath(raw);
    if (!cleaned || seen.has(cleaned)) return;
    seen.add(cleaned);
    out.push(cleaned);
  };

  if (text) {
    // 1. Markdown images and links: `![alt](target)` / `[label](target)`
    for (const match of text.matchAll(/!?\[[^\]]*\]\((<[^>\n]+>|[^)\n]+)\)/g)) {
      push(match[1] ?? "");
    }
    // 2. Inline code spans: `` `/path/to/img.png` ``
    for (const match of text.matchAll(/`([^`\n]+)`/g)) {
      push(match[1] ?? "");
    }
    // 3. Standalone lines (handles spaces inside filenames like `/Users/me/Desktop/My UI.png`)
    for (const rawLine of text.split(/\r?\n/)) {
      const stripped = rawLine
        .trim()
        .replace(/^(?:[-*•+]|\d+[.)])\s+/, "")
        .trim();
      if (
        stripped &&
        stripped.length <= 260 &&
        !/^!?\[/.test(stripped) &&
        !/[\]<>`]/.test(stripped) &&
        !stripped.includes("](") &&
        (/^(?:file:\/\/|~[/\\]|\/|[A-Za-z]:[/\\]|\.\.?[/\\])/i.test(stripped) ||
          /^[\w\u4e00-\u9fff.-]+(?:[/\\][\w\u4e00-\u9fff ._-]+)+$/i.test(stripped))
      ) {
        push(stripped);
      }
    }
    // 4. Inline local paths inside prose (`保存在 /Users/me/Desktop/效果图.png，共两张`)
    const withoutRemoteUrls = text.replace(/\bhttps?:\/\/[^\s)"'<>`]+/gi, " ");
    const inlineRe =
      /(?:file:\/\/[^\s"'<>`()，。；：！？、（）《》【】]+|(?:~[/\\]|\/|(?<![A-Za-z0-9])[A-Za-z]:[/\\]|(?:\.\.?[/\\])+)[^\s"'<>`()，。；：！？、（）《》【】]+?\.(?:png|jpe?g|gif|webp)|(?<![:/\w.-])(?:[\w\u4e00-\u9fff.-]+[/\\])+[\w\u4e00-\u9fff.-]+\.(?:png|jpe?g|gif|webp))/gi;
    for (const match of withoutRemoteUrls.matchAll(inlineRe)) {
      push(match[0] ?? "");
    }
  }

  // 5. Image files created via write tools during this turn.
  for (const file of files) {
    if (file.kind !== "output") continue;
    push(file.path);
  }

  return out;
}

export async function collectChannelReplyImages(opts: {
  text: string;
  steps?: { kind: string; text?: string }[];
  files?: { path: string; kind: "change" | "output" }[];
  cwd?: string | null;
  readImage: (path: string, cwd?: string | null) => Promise<ChannelReplyImage>;
}): Promise<ChannelReplyImage[]> {
  const stepTexts = (opts.steps ?? [])
    .filter((step) => step.kind === "text" && step.text?.trim())
    .map((step) => step.text!.trim());
  const combinedText = [...stepTexts, opts.text.trim()].filter(Boolean).join("\n");
  const candidates = extractChannelReplyImagePaths(combinedText, opts.files);
  if (candidates.length === 0) return [];

  const images: ChannelReplyImage[] = [];
  const seenPayload = new Set<string>();
  for (const candidate of candidates) {
    if (images.length >= MAX_CHANNEL_REPLY_IMAGES) break;
    try {
      const loaded = await opts.readImage(candidate, opts.cwd);
      if (!loaded?.data || !loaded.mime) continue;
      const sig = `${loaded.mime}:${loaded.data.length}:${loaded.data.slice(0, 48)}`;
      if (seenPayload.has(sig)) continue;
      seenPayload.add(sig);
      images.push(loaded);
    } catch {
      // Missing example paths or unreadable files never fail the reply itself.
    }
  }
  return images;
}
