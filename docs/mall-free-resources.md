# Mall: free storefront resources

How the free skills from the website's `/resources` page show up inside the
desktop mall (技能 → 云市场), how they install, and the invariants that came out
of the first release (2026-11-08).

## Server contract (agentdoctor.cn)

- `GET {base}/api/v1/resources/skills` — the catalog behind the website's
  `/resources` page. Shape: `{ ok, updatedAt, categories: [{ id, label, items }] }`;
  each item: `{ key, name, desc, author, downloads, detailUrl, hosted }`.
- An item is installable only when `hosted.status == "ok"`; `hosted` then carries
  `zipDownload` (site-relative URL), `sha256`, `sizeBytes`, `version`. Items with
  `hosted.status == "missing"` (「包暂缺」 on the site) have no package yet and are
  **not** listed in the mall — a row without a working Install button would be a
  dead end.
- Zip bytes: `GET {base}{zipDownload}`. The site validates structure (requires
  `SKILL.md`, no path traversal, size caps) and serves the bytes pinned to the
  snapshot. The client must not trust this and re-checks everything itself.
- `{base}` is the mall base URL (`teamups_mall_base_url`): default
  `https://agentdoctor.cn`, overridable via skills-source settings / env, same as
  the packs store.

## Catalog merge (`agent-doctor-core/src/skills/mall.rs`)

- `list_teamups_mall_catalog` fetches the packs catalog first, then merges
  `fetch_free_resources(base_url)` rows (kind `skill`, `free: true`,
  `origin: "resource"`).
- If the resources endpoint fails, the packs list still loads — the resources
  fetch must never fail the whole mall.
- Dedup: a pack already offering the same skill id wins; resource rows are
  appended only for ids not present as mall skills (`merge_free_resource_rows`).
- `installed` detection is unchanged: the directory name / lock key check in
  `local_installed_skill_ids` covers resource rows because they are kind `skill`.

## Install path (`agent-doctor-core/src/skills/resources.rs`)

`install_free_resource(id)` — used by the desktop command
`install_teamups_resource_command`, deliberately **not** routed through the
TeamUps pack sync (resources are not packs; the doctor-manifest flow would
demand a license):

1. Re-fetch the resources catalog and find the row by `key` (no client-supplied
   URLs — the server list is the source of truth, and it carries the sha256).
2. Download `{zipDownload}` (≤ 80 MB, matching the local-install budget).
3. Verify sha256 when the catalog provides one (case-insensitive).
4. Extract into `skills_dir/{key}` with the shared zip safety
   (`extract_zip_bytes` + `normalize_skill_extract_dir`), then the fence: no
   `SKILL.md` after normalize ⇒ delete the target and fail with an
   `invalid skill zip` message (the friendly-error classifier maps it to the
   broken-package hint). Failed installs leave nothing behind.
5. Record the lock entry (`skills.{key}`: version, sha256, installed_at,
   `source: "resources"`). **Never touch `bundle_id`** — a resource install must
   not hijack an installed pack's lock identity (regression-guarded by
   `installs_resource_zip_and_records_lock_without_bundle_id`).

## UI invariants (desktop)

One-click install:

- **Install means install.** Tapping 「安装」 on a single skill (storefront
  resource or single TeamUps skill) downloads it *and* mounts it on the local
  agents (`mountSkill`) in one action. The row then shows mounted agent chips —
  there is no second step.
- **Two actions, two words.** 「安装」/Install is store download (now including
  mount); 「添加到 Agent」/Add to agents is mount-only for skills already on this
  computer. Never show two install-flavored labels for different actions — that
  pairing read as two install methods and misled users (2026-11-08 feedback).
- Packs still land without mounting (batch mounting is a heavy action); their
  success message points at 「添加到 Agent」 on the skill rows.

Row collapse layout:

- The 展开/收起 toggle lives on the description's line (`res-catalog-desc-row`,
  flex, bottom-aligned) and must never wrap onto its own line: the description is
  `line-clamp`ed and shrinks so the toggle always fits. A toggle on its own line
  wastes a row and read as broken (2026-11-08 feedback).
- Storefront rows use the site's category label (用助手 …) as the badge and a
  plain 免费/已安装 meta; no download counts or author names in the row meta
  (kept minimal on purpose).

## Dev verification notes (from this session)

- **Verify rendering with a rendered page.** Layout changes get a throwaway page
  (e.g. `desktop/row-test.html`) that links the real stylesheet, mirrors the exact
  DOM the row builder creates, and is screenshotted in a browser before claiming
  the fix works. `tsc` passing is not visual verification.
- **Stale HMR looks like a code bug.** Vite's file watcher can silently miss a
  file (`resources-skills.ts` missed every edit in one session): the window keeps
  old JS while CSS hot-updates apply, producing hybrids (a button stretching
  centered across a grid cell) that no code path explains. When screenshots don't
  match the code on disk, restart the dev session instead of debugging phantom
  bugs; `curl :1420/src/<file>` shows what the server would serve.
- **A dev app without its dev server is a white window.** `tauri dev` builds load
  `http://127.0.0.1:1420`; if the CLI died but the `.app` survived, every window
  is blank. Fix: kill the orphan and relaunch `npm run tauri:dev:personal` — do
  not touch frontendDist for this.
