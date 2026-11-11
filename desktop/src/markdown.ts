/** Lightweight GFM-ish markdown → HTML. Escapes raw HTML first (safe for LLM output). */

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

export type MarkdownOptions = {
  /** Turn `[text](page.md)` and `[[Page]]` into in-page links carrying `data-wiki-page` / `data-wiki-name`. */
  wikiLinks?: boolean;
};

function inlineMarkdown(text: string, opts: MarkdownOptions = {}): string {
  let s = escapeHtml(text);
  // code
  s = s.replace(/`([^`]+)`/g, "<code>$1</code>");
  // bold / italic (order matters)
  s = s.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  s = s.replace(/__([^_]+)__/g, "<strong>$1</strong>");
  s = s.replace(/\*([^*\n]+)\*/g, "<em>$1</em>");
  s = s.replace(/_([^_\n]+)_/g, "<em>$1</em>");
  // links [text](url) — only http(s)
  s = s.replace(
    /\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g,
    '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>',
  );
  if (opts.wikiLinks) {
    s = s.replace(/\[([^\]]+)\]\(([^)\s#]+\.md)(?:#[^)\s]*)?\)/g, '<a href="#" data-wiki-page="$2">$1</a>');
    s = s.replace(
      /\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g,
      (_all, name: string, label?: string) => `<a href="#" data-wiki-name="${name.trim()}">${(label ?? name).trim()}</a>`,
    );
  }
  return s;
}

/**
 * Last block boundary safe to freeze: outside a code fence, on a blank line,
 * and not between two items of one list/quote/table (that would split the
 * block). Paseo only re-parses streaming markdown up to such an edge — the
 * completed prefix renders once and every tick repaints just the tail.
 *
 * `fenced` reports an unclosed fence at the cut. A streaming fence has no
 * safe boundary inside it, so the tail keeps re-parsing the whole fence on
 * every tick; the caller renders that tail as plain code text instead.
 */
export function markdownCutInfo(raw: string): { cut: number; fenced: boolean } {
  const isContinuation = (line: string): boolean =>
    /^\s{0,3}(?:[-*+]\s|\d+[.)]\s|>\s?|\|)/.test(line);
  const lines = raw.split("\n");
  let fence = false;
  let cut = 0;
  let offset = 0;
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const start = offset;
    offset += line.length + 1;
    if (/^```/.test(line)) fence = !fence;
    if (fence || line.trim() !== "") continue;
    let prev = i - 1;
    while (prev >= 0 && lines[prev].trim() === "") prev -= 1;
    let j = i + 1;
    while (j < lines.length && lines[j].trim() === "") j += 1;
    if (j >= lines.length) {
      cut = raw.length;
      break;
    }
    if (isContinuation(lines[j]) && prev >= 0 && isContinuation(lines[prev])) continue;
    cut = start;
  }
  return { cut, fenced: fence };
}

export function safeMarkdownCut(raw: string): number {
  return markdownCutInfo(raw).cut;
}

/**
 * Convert markdown source to sanitized HTML (no raw HTML passthrough).
 */
export function renderMarkdown(source: string, opts: MarkdownOptions = {}): string {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  let i = 0;
  let inCode: string | null = null;
  let codeBuf: string[] = [];
  let listType: "ul" | "ol" | null = null;

  const closeList = () => {
    if (listType) {
      out.push(`</${listType}>`);
      listType = null;
    }
  };

  const isTableSep = (line: string): boolean =>
    /^\s*\|?[\s:|-]+\|[\s:|-]*\|?\s*$/.test(line) && /---/.test(line);

  const splitTableRow = (line: string): string[] => {
    let row = line.trim();
    if (row.startsWith("|")) row = row.slice(1);
    if (row.endsWith("|")) row = row.slice(0, -1);
    return row.split("|").map((cell) => cell.trim());
  };

  while (i < lines.length) {
    const line = lines[i];

    const fence = line.match(/^```([\w-]*)\s*$/);
    if (fence) {
      if (inCode != null) {
        out.push(
          `<pre><code class="language-${escapeHtml(inCode)}">${escapeHtml(codeBuf.join("\n"))}</code></pre>`,
        );
        inCode = null;
        codeBuf = [];
      } else {
        closeList();
        inCode = fence[1] || "text";
        codeBuf = [];
      }
      i += 1;
      continue;
    }

    if (inCode != null) {
      codeBuf.push(line);
      i += 1;
      continue;
    }

    if (/^\s*$/.test(line)) {
      closeList();
      i += 1;
      continue;
    }

    // GFM table: header | sep | rows…
    if (
      line.includes("|") &&
      i + 1 < lines.length &&
      isTableSep(lines[i + 1])
    ) {
      closeList();
      const header = splitTableRow(line);
      i += 2;
      const body: string[][] = [];
      while (i < lines.length && lines[i].includes("|") && lines[i].trim()) {
        if (isTableSep(lines[i])) {
          i += 1;
          continue;
        }
        body.push(splitTableRow(lines[i]));
        i += 1;
      }
      const thead = `<thead><tr>${header
        .map((cell) => `<th>${inlineMarkdown(cell, opts)}</th>`)
        .join("")}</tr></thead>`;
      const tbody =
        body.length > 0
          ? `<tbody>${body
              .map(
                (row) =>
                  `<tr>${row.map((cell) => `<td>${inlineMarkdown(cell, opts)}</td>`).join("")}</tr>`,
              )
              .join("")}</tbody>`
          : "";
      out.push(`<div class="chat-md-table-wrap"><table>${thead}${tbody}</table></div>`);
      continue;
    }

    const heading = line.match(/^(#{1,3})\s+(.+)$/);
    if (heading) {
      closeList();
      const level = heading[1].length;
      out.push(`<h${level}>${inlineMarkdown(heading[2].trim(), opts)}</h${level}>`);
      i += 1;
      continue;
    }

    if (/^>\s?/.test(line)) {
      closeList();
      const quote: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) {
        quote.push(lines[i].replace(/^>\s?/, ""));
        i += 1;
      }
      out.push(`<blockquote>${inlineMarkdown(quote.join(" "), opts)}</blockquote>`);
      continue;
    }

    const ul = line.match(/^[-*+]\s+(.+)$/);
    if (ul) {
      if (listType !== "ul") {
        closeList();
        listType = "ul";
        out.push("<ul>");
      }
      out.push(`<li>${inlineMarkdown(ul[1], opts)}</li>`);
      i += 1;
      continue;
    }

    const ol = line.match(/^\d+\.\s+(.+)$/);
    if (ol) {
      if (listType !== "ol") {
        closeList();
        listType = "ol";
        out.push("<ol>");
      }
      out.push(`<li>${inlineMarkdown(ol[1], opts)}</li>`);
      i += 1;
      continue;
    }

    if (/^---+$/.test(line.trim())) {
      closeList();
      out.push("<hr />");
      i += 1;
      continue;
    }

    closeList();
    const para: string[] = [line];
    i += 1;
    while (
      i < lines.length &&
      lines[i].trim() &&
      !/^(#{1,3}\s|[-*+]\s|\d+\.\s|>\s?|```|---+$)/.test(lines[i]) &&
      !(lines[i].includes("|") && i + 1 < lines.length && isTableSep(lines[i + 1]))
    ) {
      para.push(lines[i]);
      i += 1;
    }
    out.push(`<p>${inlineMarkdown(para.join("\n"), opts).replace(/\n/g, "<br />")}</p>`);
  }

  if (inCode != null) {
    out.push(`<pre><code>${escapeHtml(codeBuf.join("\n"))}</code></pre>`);
  }
  closeList();
  return out.join("");
}
