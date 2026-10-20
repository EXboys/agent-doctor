#!/usr/bin/env python3
"""Discover mirrored skills from listing metadata and write their addresses.

Reads the SkillHub list (name, author, downloads, category, upstream link).
Does not download skill files. The catalog stores the mirror address.
"""

from __future__ import annotations

import json
import urllib.parse
import urllib.request
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SITE = ROOT.parent
API = "https://api.skillhub.cn"
PER_GROUP = 6
MAX_PAGES = 25
PAGE_SIZE = 20
# Used only when the download ranking has not filled a group.
FILL_QUERIES = {
    "geo": ["ai-seo", "generative engine"],
    "video": ["video generation"],
    "slides": ["pptx"],
    "writing": ["copywriting"],
    "code": ["react best practices"],
}

# First matching subcategory wins. Category is the fallback.
GROUPS = [
    ("writing", "Articles", "写文章"),
    ("code", "Code", "写代码"),
    ("video", "Video", "做视频"),
    ("slides", "Slides", "做 PPT"),
    ("geo", "GEO", "做 GEO"),
    ("docs", "Documents", "写文档"),
    ("data", "Data", "看数据"),
    ("design", "Design", "做设计"),
    ("notes", "Notes", "查资料"),
    ("agents", "Agents", "用助手"),
]
GROUP_BY_ID = {item[0]: item for item in GROUPS}

SUBCATEGORY_GROUP = {
    "content-article": "writing",
    "content-rewrite": "writing",
    "content-marketing-copy": "writing",
    "content-title": "writing",
    "dev-code-gen": "code",
    "dev-code-review": "code",
    "dev-frontend": "code",
    "dev-script": "code",
    "dev-git": "code",
    "design-video": "video",
    "design-audio": "video",
    "office-ppt": "slides",
    "office-doc": "docs",
    "office-pdf": "docs",
    "office-spreadsheet": "docs",
    "office-email": "writing",
    "data-insight": "data",
    "data-report": "data",
    "data-visualization": "data",
    "design-ui": "design",
    "design-image-gen": "design",
    "design-image-edit": "design",
    "design-visual-asset": "design",
    "knowledge-retrieval": "notes",
    "knowledge-summary": "notes",
    "knowledge-doc-qa": "notes",
    "knowledge-base-qa": "notes",
    "knowledge-organize": "notes",
    "agent-memory": "agents",
    "agent-context": "agents",
    "agent-workflow": "agents",
    "agent-task-automation": "agents",
    "agent-tool-use": "agents",
    "agent-prompt": "agents",
    "agent-framework": "agents",
    "agent-multi-agent": "agents",
}
CATEGORY_GROUP = {
    "content-creation": "writing",
    "dev-programming": "code",
    "design-media": "design",
    "office-efficiency": "docs",
    "data-analysis": "data",
    "ai-agent": "agents",
    "knowledge-management": "notes",
}
FITS = {
    "content-article": "Articles and pages",
    "content-rewrite": "Rewriting a draft",
    "content-marketing-copy": "Pages and product lines",
    "dev-code-gen": "Writing code",
    "dev-code-review": "Reviewing code",
    "dev-frontend": "Pages and components",
    "dev-script": "Scripts",
    "design-video": "Explainers and short clips",
    "design-audio": "Voice and sound",
    "office-ppt": "Reports, talks, pitches",
    "office-doc": "Documents",
    "office-pdf": "PDF files",
    "office-spreadsheet": "Spreadsheets",
    "office-email": "Email",
    "data-insight": "Numbers and charts",
    "data-report": "Reports",
    "design-ui": "Screens and layout",
    "design-image-gen": "Pictures",
    "knowledge-summary": "Summaries",
    "knowledge-retrieval": "Finding a note",
    "agent-task-automation": "Repeated jobs",
    "agent-tool-use": "Using a tool",
    "agent-memory": "Remembering a job",
}


def get_json(url: str) -> dict:
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "agent-doctor", "Accept": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def needs_key(skill: dict) -> bool:
    labels = skill.get("labels") or {}
    return str(labels.get("requires_api_key", "")).lower() == "true"


def classify(skill: dict) -> tuple[str, str] | None:
    text = " ".join(
        [
            skill.get("slug") or "",
            skill.get("name") or "",
            skill.get("description") or "",
        ]
    ).lower()
    if any(token in text for token in ("ai-seo", " llms.txt", "generative engine", "geo ")):
        return "geo", "GEO, AI Overviews, llms.txt"
    subs = skill.get("subCategories") or []
    for sub in subs:
        group = SUBCATEGORY_GROUP.get(sub.get("key") or "")
        if group:
            return group, FITS.get(sub.get("key") or "", "")
    group = CATEGORY_GROUP.get(skill.get("category") or "")
    if not group:
        return None
    return group, ""


def summary(skill: dict) -> str:
    text = (skill.get("description") or skill.get("description_zh") or "").replace("\n", " ")
    text = " ".join(text.split())
    if len(text) <= 180:
        return text
    cut = text[:180].rsplit(" ", 1)[0]
    return cut.rstrip(".,;:") + "…"


def downloads_label(count: int) -> str:
    if count >= 1_000_000:
        return f"{count / 1_000_000:.1f}M downloads"
    if count >= 1_000:
        return f"{count / 1_000:.1f}K downloads"
    return f"{count} downloads"


def consider(chosen: dict[str, list[dict]], seen: set[str], skill: dict) -> None:
    slug = (skill.get("slug") or "").strip()
    upstream = (skill.get("upstream_url") or "").strip()
    if not slug or slug in seen or not upstream or needs_key(skill):
        return
    classified = classify(skill)
    if not classified:
        return
    group, fits = classified
    bucket = chosen[group]
    if len(bucket) >= PER_GROUP:
        return
    seen.add(slug)
    owner = skill.get("upstream_owner_login") or skill.get("ownerName") or ""
    bucket.append(
        {
            "id": slug,
            "name": skill.get("name") or slug,
            "skill": slug,
            "zh": GROUP_BY_ID[group][2],
            "group": group,
            "groupLabel": GROUP_BY_ID[group][1],
            "summary": summary(skill),
            "fits": fits or GROUP_BY_ID[group][1],
            "author": owner,
            "source": upstream,
            "installs": downloads_label(int(skill.get("downloads") or 0)),
            "downloads": int(skill.get("downloads") or 0),
            "download": f"{API}/api/v1/download?slug={urllib.parse.quote(slug)}",
        }
    )


def discover() -> list[dict]:
    chosen: dict[str, list[dict]] = defaultdict(list)
    seen: set[str] = set()
    for page in range(1, MAX_PAGES + 1):
        if all(len(chosen[group[0]]) >= PER_GROUP for group in GROUPS):
            break
        payload = get_json(
            f"{API}/api/skills?"
            + urllib.parse.urlencode(
                {
                    "source": "clawhub",
                    "sortBy": "downloads",
                    "order": "desc",
                    "pageSize": PAGE_SIZE,
                    "page": page,
                }
            )
        )
        skills = payload.get("data", {}).get("skills") or []
        if not skills:
            break
        for skill in skills:
            consider(chosen, seen, skill)
    for group, queries in FILL_QUERIES.items():
        if len(chosen[group]) >= PER_GROUP:
            continue
        for query in queries:
            if len(chosen[group]) >= PER_GROUP:
                break
            payload = get_json(
                f"{API}/api/skills?"
                + urllib.parse.urlencode(
                    {
                        "source": "clawhub",
                        "keyword": query,
                        "sortBy": "downloads",
                        "order": "desc",
                        "pageSize": PAGE_SIZE,
                    }
                )
            )
            for skill in payload.get("data", {}).get("skills") or []:
                consider(chosen, seen, skill)
    items = []
    for group, _label, _zh in GROUPS:
        ranked = sorted(
            chosen[group],
            key=lambda item: item["downloads"],
            reverse=True,
        )
        items.extend(ranked[:PER_GROUP])
    return items


def write_catalog(items: list[dict]) -> None:
    present = []
    seen = set()
    for item in items:
        if item["group"] in seen:
            continue
        seen.add(item["group"])
        group_id, label, zh = GROUP_BY_ID[item["group"]]
        present.append({"id": group_id, "label": label, "zh": zh})
    catalog = {"groups": present, "items": items}
    (ROOT / "catalog.json").write_text(
        json.dumps(catalog, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


def write_redirects(items: list[dict]) -> None:
    lines = [
        "/api/v1/packs /skills/catalog.json 200",
        "/api/v1/skills /skills/catalog.json 200",
    ]
    legacy = {
        "copywriting": "https://www.modelscope.cn/skills/@coreyhaines31/copywriting/archive/zip/master",
        "vercel-react-best-practices": "https://www.modelscope.cn/skills/@vercel-labs/react-best-practices/archive/zip/master",
        "video": "https://clawhub.ai/api/v1/download?slug=video&ownerHandle=coreyhaines31",
        "codex-ppt": "https://api.skillhub.cn/api/v1/download?slug=codex-ppt",
        "ai-seo": "https://www.modelscope.cn/skills/@coreyhaines31/ai-seo/archive/zip/master",
    }
    targets = {item["id"]: item["download"] for item in items}
    targets.update(legacy)
    for slug, url in targets.items():
        lines.append(f"/skills/{slug}/{slug}.zip {url} 302")
        lines.append(f"/api/v1/packs/{slug}/skills/{slug} {url} 302")
        lines.append(f"/skills/{slug}/manifest.json {url} 302")
    (SITE / "_redirects").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main() -> None:
    items = discover()
    if not items:
        raise SystemExit("no skills passed scoring")
    write_catalog(items)
    write_redirects(items)
    print(f"published {len(items)} skills")


if __name__ == "__main__":
    main()
