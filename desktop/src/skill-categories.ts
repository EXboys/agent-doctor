/**
 * Heuristic skill categories for the Resources catalog.
 * Keep rules here so UI only consumes category ids + labels.
 */

export type SkillCategoryId =
  | "dev"
  | "web"
  | "data"
  | "docs"
  | "office"
  | "design"
  | "ops"
  | "ai"
  | "other";

export type SkillCategoryDef = {
  id: SkillCategoryId;
  /** i18n message key under resources.cat.* */
  labelKey: string;
  /** Match against `${id} ${name} ${description}`. First hit wins. */
  match: RegExp;
};

/** Ordered rules — more specific categories first. */
export const SKILL_CATEGORY_RULES: SkillCategoryDef[] = [
  {
    id: "web",
    labelKey: "resources.catWeb",
    match:
      /\b(browser|chrome|playwright|puppeteer|selenium|scrape|crawl|fetch\s*url|http\s*request|网页|浏览器|爬虫|抓取)\b/i,
  },
  {
    id: "data",
    labelKey: "resources.catData",
    match:
      /\b(sql|database|postgres|mysql|sqlite|mongo|redis|excel|csv|dataframe|pandas|etl|数据|表格|数据库|统计|分析)\b/i,
  },
  {
    id: "docs",
    labelKey: "resources.catDocs",
    match:
      /\b(markdown|readme|doc(?:s|ument)?|pdf|notion|confluence|wiki|写作|文档|笔记|总结|摘要)\b/i,
  },
  {
    id: "office",
    labelKey: "resources.catOffice",
    match:
      /\b(email|mail|calendar|slack|teams|jira|linear|airtable|notion|apple[-_ ]?notes|reminders|办公|邮件|日历|会议|待办)\b/i,
  },
  {
    id: "design",
    labelKey: "resources.catDesign",
    match: /\b(figma|design|ui|ux|image|svg|icon|画|设计|视觉|原型)\b/i,
  },
  {
    id: "ops",
    labelKey: "resources.catOps",
    match:
      /\b(docker|k8s|kubernetes|ci|cd|deploy|monitor|log|sre|devops|运维|部署|监控|日志)\b/i,
  },
  {
    id: "ai",
    labelKey: "resources.catAi",
    match:
      /\b(llm|gpt|claude|embedding|rag|prompt|agent|model|ml|ai|模型|智能体|提示词)\b/i,
  },
  {
    id: "dev",
    labelKey: "resources.catDev",
    match:
      /\b(git|github|gitlab|code|debug|test|lint|npm|node|rust|python|typescript|api|sdk|refactor|开发|代码|编程|调试|测试)\b/i,
  },
  {
    id: "other",
    labelKey: "resources.catOther",
    match: /./,
  },
];

export const SKILL_CATEGORY_ORDER: SkillCategoryId[] = SKILL_CATEGORY_RULES.map((r) => r.id);

export function classifySkillCategory(input: {
  id?: string | null;
  name?: string | null;
  description?: string | null;
}): SkillCategoryId {
  const haystack = [input.id, input.name, input.description]
    .map((part) => (part ?? "").trim())
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  if (!haystack) return "other";
  for (const rule of SKILL_CATEGORY_RULES) {
    if (rule.id === "other") continue;
    if (rule.match.test(haystack)) return rule.id;
  }
  return "other";
}

export function skillCategoryLabelKey(id: SkillCategoryId): string {
  return SKILL_CATEGORY_RULES.find((rule) => rule.id === id)?.labelKey ?? "resources.catOther";
}
