import { escapeHtml } from "../markdown";

const KEYWORDS = new Set(
  [
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "crate",
    "def",
    "default",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "extern",
    "false",
    "finally",
    "fn",
    "for",
    "from",
    "function",
    "if",
    "impl",
    "import",
    "in",
    "interface",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "new",
    "None",
    "null",
    "of",
    "override",
    "package",
    "pass",
    "private",
    "protected",
    "pub",
    "public",
    "readonly",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "switch",
    "this",
    "throw",
    "trait",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "use",
    "var",
    "void",
    "where",
    "while",
    "with",
    "yield",
  ],
);

const HASH_COMMENT_LANGUAGES = new Set([
  "python",
  "bash",
  "shell",
  "yaml",
  "toml",
  "ruby",
]);

function tokenClass(token: string, source: string, end: number): string | null {
  if (
    token.startsWith("//") ||
    token.startsWith("/*") ||
    token.startsWith("#") ||
    token.startsWith("<!--") ||
    token.startsWith("--")
  ) {
    return "syntax-comment";
  }
  if (token.startsWith('"') || token.startsWith("'") || token.startsWith("`")) {
    return "syntax-string";
  }
  if (/^\d/.test(token)) return "syntax-number";
  if (KEYWORDS.has(token)) return "syntax-keyword";
  if (/^(true|false|null|undefined|None)$/.test(token)) return "syntax-literal";
  if (/^\s*\(/.test(source.slice(end))) return "syntax-function";
  return null;
}

function highlightMarkdown(source: string): string {
  return source
    .split("\n")
    .map((line) => {
      const heading = line.match(/^(\s{0,3})(#{1,6})(\s+)(.*)$/);
      if (heading) {
        return `${escapeHtml(heading[1])}<span class="syntax-marker">${escapeHtml(heading[2])}</span>${escapeHtml(heading[3])}<span class="syntax-heading">${escapeHtml(heading[4])}</span>`;
      }
      const fence = line.match(/^(\s*)(```+)(.*)$/);
      if (fence) {
        return `${escapeHtml(fence[1])}<span class="syntax-marker">${escapeHtml(fence[2])}</span><span class="syntax-keyword">${escapeHtml(fence[3])}</span>`;
      }
      const quote = line.match(/^(\s*)(>)(\s?.*)$/);
      if (quote) {
        return `${escapeHtml(quote[1])}<span class="syntax-marker">${escapeHtml(quote[2])}</span><span class="syntax-comment">${escapeHtml(quote[3])}</span>`;
      }
      let escaped = escapeHtml(line);
      escaped = escaped.replace(
        /(`[^`]*`)/g,
        '<span class="syntax-string">$1</span>',
      );
      escaped = escaped.replace(
        /^(\s*)([-*+]|\d+\.)(\s+)/,
        '$1<span class="syntax-marker">$2</span>$3',
      );
      return escaped;
    })
    .join("\n");
}

const PLAIN_FENCE_LANGUAGES = new Set(["", "text", "plain", "plaintext", "txt"]);

/** Vertical wheel stays in the code box only while that box can still move. */
export function verticalWheelStaysInCodeBlock(deltaY: number, scrollTop: number, maxScrollTop: number): boolean {
  if (Math.abs(deltaY) < 0.5 || maxScrollTop <= 1) return false;
  if (deltaY < 0) return scrollTop > 0;
  return scrollTop < maxScrollTop - 1;
}

/** Fences with no language, or marked as plain text, stay uncolored and wrap. */
export function isPlainFenceLanguage(language: string): boolean {
  return PLAIN_FENCE_LANGUAGES.has(language.trim().toLowerCase());
}

/** Safe, lightweight syntax coloring for file preview and chat code fences. */
export function highlightCode(source: string, language: string): string {
  const normalized = language.toLowerCase();
  if (normalized === "markdown") return highlightMarkdown(source);

  const comments = HASH_COMMENT_LANGUAGES.has(normalized)
    ? String.raw`#[^\n]*`
    : normalized === "html" || normalized === "xml" || normalized === "vue"
      ? String.raw`<!--[\s\S]*?-->`
      : normalized === "sql"
        ? String.raw`--[^\n]*|\/\*[\s\S]*?\*\/`
        : String.raw`\/\/[^\n]*|\/\*[\s\S]*?\*\/`;
  const matcher = new RegExp(
    `${comments}|"(?:\\\\.|[^"\\\\])*"|'(?:\\\\.|[^'\\\\])*'|\\\`(?:\\\\.|[^\\\`\\\\])*\\\`|\\b\\d+(?:\\.\\d+)?\\b|\\b[A-Za-z_$][\\w$]*\\b`,
    "g",
  );

  let html = "";
  let cursor = 0;
  for (const match of source.matchAll(matcher)) {
    const start = match.index ?? 0;
    const token = match[0];
    const end = start + token.length;
    html += escapeHtml(source.slice(cursor, start));
    const className = tokenClass(token, source, end);
    html += className
      ? `<span class="${className}">${escapeHtml(token)}</span>`
      : escapeHtml(token);
    cursor = end;
  }
  html += escapeHtml(source.slice(cursor));
  return html;
}
