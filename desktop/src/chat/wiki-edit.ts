/** Turn a knowledge page the user is editing back into markdown. */

/** Matches `MAX_PAGE_BYTES` in the desktop knowledge command. */
export const WIKI_PAGE_MAX_BYTES = 512 * 1024;

const TEXT = 3;
const ELEMENT = 1;

export function splitWikiPage(markdown: string): { title: string; body: string } {
  const text = markdown.replace(/^\uFEFF/, "").replace(/\r\n/g, "\n");
  const lines = text.split("\n");
  let start = 0;
  while (start < lines.length && !lines[start].trim()) start += 1;
  const heading = lines[start]?.match(/^#\s+(.+)$/);
  if (!heading) return { title: "", body: text.trim() };
  return {
    title: heading[1].trim(),
    body: lines.slice(start + 1).join("\n").replace(/^\n/, "").trim(),
  };
}

export function joinWikiPage(title: string, body: string): string {
  const heading = `# ${title.replace(/[\r\n]+/g, " ").trim()}`;
  const rest = body.replace(/\r\n/g, "\n").trim();
  return rest ? `${heading}\n\n${rest}\n` : `${heading}\n`;
}

export function nextNotePath(existing: string[]): string {
  const used = new Set(existing);
  if (!used.has("notes/note.md")) return "notes/note.md";
  let n = 2;
  while (used.has(`notes/note-${n}.md`)) n += 1;
  return `notes/note-${n}.md`;
}

/** First plain sentence, for a new line in the index. */
export function plainLead(markdown: string): string {
  const line = markdown
    .split("\n")
    .map((item) => item.trim())
    .find((item) => item && !item.startsWith("#") && !item.startsWith("```") && !item.startsWith("|") && item !== "---");
  if (!line) return "";
  const plain = line
    .replace(/^[-*+]\s+/, "")
    .replace(/^\d+\.\s+/, "")
    .replace(/^>\s?/, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/\[\[[^\]|]+\|([^\]]+)\]\]/g, "$1")
    .replace(/\[\[([^\]]+)\]\]/g, "$1")
    .replace(/[`*_]/g, "")
    .replace(/[\[\]]/g, "")
    .trim();
  if (!plain) return "";
  return plain.length > 80 ? `${plain.slice(0, 80)}…` : plain;
}

/**
 * Keep the index link in step with the page title.
 * An existing line keeps its summary. A missing page is appended.
 * `relabel` false leaves an existing link's text alone.
 */
export function upsertIndexLink(index: string, path: string, title: string, summary = "", relabel = true): string {
  const safeTitle = title.replace(/[\[\]]/g, "").trim();
  if (!safeTitle || path === "index.md" || path === "log.md") return index;
  const escaped = path.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = `\\[[^\\]\\n]*\\]\\(${escaped}\\)`;
  if (new RegExp(pattern).test(index)) {
    if (!relabel) return index;
    const link = `[${safeTitle}](${path})`;
    return index.replace(new RegExp(pattern, "g"), () => link);
  }
  const extra = summary.replace(/[\[\]]/g, "").replace(/\s+/g, " ").trim();
  const line = extra ? `- [${safeTitle}](${path}) — ${extra}` : `- [${safeTitle}](${path})`;
  const base = index.replace(/\s*$/, "");
  return base ? `${base}\n${line}\n` : `${line}\n`;
}

function kids(node: Node): Node[] {
  const out: Node[] = [];
  const list = node.childNodes;
  for (let i = 0; i < list.length; i += 1) out.push(list[i]);
  return out;
}

function elements(node: Element): Element[] {
  const out: Element[] = [];
  const list = node.children;
  for (let i = 0; i < list.length; i += 1) out.push(list[i]);
  return out;
}

function isEl(node: Node): node is Element {
  return node.nodeType === ELEMENT;
}

function tagOf(node: Element): string {
  return node.tagName.toUpperCase();
}

const BLOCKS = new Set(["H1", "H2", "H3", "H4", "H5", "H6", "P", "DIV", "UL", "OL", "PRE", "BLOCKQUOTE", "HR", "TABLE"]);

function isBlock(node: Element): boolean {
  return BLOCKS.has(tagOf(node));
}

function inlineToMd(node: Node): string {
  if (node.nodeType === TEXT) return (node.textContent ?? "").replace(/\u00a0/g, " ");
  if (!isEl(node)) return "";
  const tag = tagOf(node);
  if (tag === "BR") return "\n";
  if (tag === "SCRIPT" || tag === "STYLE" || tag === "UL" || tag === "OL" || tag === "PRE" || tag === "TABLE") return "";
  const inner = kids(node).map(inlineToMd).join("");
  if (tag === "STRONG" || tag === "B") return inner.trim() ? `**${inner.trim()}**` : "";
  if (tag === "EM" || tag === "I") return inner.trim() ? `*${inner.trim()}*` : "";
  if (tag === "CODE") return inner ? `\`${inner}\`` : "";
  if (tag === "A") return linkToMd(node, inner.trim());
  return inner;
}

function linkToMd(node: Element, label: string): string {
  const wiki = node.getAttribute("data-wiki-page");
  if (wiki) return `[${label}](${wiki})`;
  const name = node.getAttribute("data-wiki-name")?.trim();
  if (name) return label === name ? `[[${name}]]` : `[[${name}|${label}]]`;
  const href = node.getAttribute("href") ?? "";
  if (href && href !== "#") return `[${label}](${href})`;
  return label;
}

function listToMd(list: Element): string {
  const ordered = tagOf(list) === "OL";
  return elements(list)
    .filter((item) => tagOf(item) === "LI")
    .map((item, index) => {
      const nested = elements(item).filter((child) => tagOf(child) === "UL" || tagOf(child) === "OL");
      const text = inlineToMd(item).replace(/\n{3,}/g, "\n\n").trim();
      const extra = nested.map(listToMd).filter(Boolean).join("\n");
      const body = [text, extra].filter(Boolean).join("\n");
      const marker = ordered ? `${index + 1}. ` : "- ";
      return body
        .split("\n")
        .map((line, lineIndex) => (lineIndex === 0 ? `${marker}${line}` : `  ${line}`))
        .join("\n");
    })
    .join("\n");
}

function tableToMd(table: Element): string {
  const rows: Element[] = [];
  for (const section of elements(table)) {
    const sectionTag = tagOf(section);
    if (sectionTag === "TR") rows.push(section);
    else if (sectionTag === "THEAD" || sectionTag === "TBODY" || sectionTag === "TFOOT") {
      for (const row of elements(section)) {
        if (tagOf(row) === "TR") rows.push(row);
      }
    }
  }
  if (!rows.length) return "";
  const cells = rows.map((row) =>
    elements(row)
      .filter((cell) => tagOf(cell) === "TH" || tagOf(cell) === "TD")
      .map((cell) => inlineToMd(cell).replace(/\|/g, "\\|").replace(/\n/g, " ").trim()),
  );
  const width = Math.max(...cells.map((row) => row.length));
  if (!width) return "";
  const line = (row: string[]) => {
    const padded = row.slice();
    while (padded.length < width) padded.push("");
    return `| ${padded.join(" | ")} |`;
  };
  return [line(cells[0]), `| ${Array(width).fill("---").join(" | ")} |`, ...cells.slice(1).map(line)].join("\n");
}

function blockToMd(node: Element): string {
  const tag = tagOf(node);
  if (tag === "SCRIPT" || tag === "STYLE") return "";
  if (/^H[1-6]$/.test(tag)) {
    const level = Math.min(3, Number(tag.slice(1)));
    const text = inlineToMd(node).replace(/\s+/g, " ").trim();
    return text ? `${"#".repeat(level)} ${text}` : "";
  }
  if (tag === "UL" || tag === "OL") return listToMd(node);
  if (tag === "PRE") {
    const code = node.querySelector("code");
    const raw = (code?.textContent ?? node.textContent ?? "").replace(/\u00a0/g, " ").replace(/\n$/, "");
    const lang = code?.getAttribute("class")?.match(/language-([\w+-]+)/)?.[1] ?? "";
    const fence = lang && lang !== "text" ? lang : "";
    return `\`\`\`${fence}\n${raw}\n\`\`\``;
  }
  if (tag === "BLOCKQUOTE") {
    const text = inlineToMd(node).trim();
    return text ? text.split("\n").map((line) => `> ${line}`).join("\n") : "";
  }
  if (tag === "HR") return "---";
  if (tag === "TABLE") return tableToMd(node);
  if (tag === "DIV" && node.classList.contains("chat-md-table-wrap")) {
    const table = node.querySelector("table");
    return table ? tableToMd(table) : "";
  }
  if (tag === "DIV") {
    const meaningful = kids(node).filter((child) => isEl(child) || (child.textContent ?? "").trim());
    const blocks = meaningful.filter((child) => isEl(child) && isBlock(child));
    if (blocks.length && blocks.length === meaningful.filter(isEl).length) return htmlToMarkdown(node);
    return inlineToMd(node).trim();
  }
  return inlineToMd(node).trim();
}

/** Serialize the editable page body. The title is stored separately. */
export function htmlToMarkdown(root: Node): string {
  const blocks: string[] = [];
  let inline = "";
  const flush = () => {
    const text = inline.replace(/[ \t]+\n/g, "\n").trim();
    inline = "";
    if (text) blocks.push(text);
  };
  for (const child of kids(root)) {
    if (child.nodeType === TEXT) {
      if (!(child.textContent ?? "").trim()) continue;
      inline += inlineToMd(child);
      continue;
    }
    if (!isEl(child)) continue;
    if (isBlock(child)) {
      flush();
      const md = blockToMd(child).replace(/[ \t]+$/g, "");
      if (md.trim()) blocks.push(md);
      continue;
    }
    inline += inlineToMd(child);
  }
  flush();
  return blocks.join("\n\n").replace(/\n{3,}/g, "\n\n");
}
