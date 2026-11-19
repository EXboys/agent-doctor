#!/usr/bin/env bash
# Build exo's Svelte dashboard (optionally with Chinese overlays) and install it
# next to the managed runtime so Agent Doctor can pass EXO_DASHBOARD_DIR.
#
# Usage:
#   ./scripts/build-exo-dashboard-zh.sh              # build + install to ~/.agent-doctor/...
#   ./scripts/build-exo-dashboard-zh.sh --no-install # only build into cache
#   ./scripts/build-exo-dashboard-zh.sh --english-only
#
# Pin must stay in sync with crates/agent-doctor-cluster PINNED_EXO_VERSION.
#
# Build cache (upstream git checkout + node_modules) lives OUTSIDE the repo under
# ~/.agent-doctor/cache/… so nested exo/.git cannot confuse Cursor/git about this workspace.
# Only the small overlay TSV stays in-repo: vendor/exo-dashboard-zh/
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PINNED_VERSION="${AGENT_DOCTOR_EXO_VERSION:-1.0.71}"
# App releases are tagged vX.Y.Z; fall back to main only when the tag is missing.
EXO_REF="${AGENT_DOCTOR_EXO_REF:-v${PINNED_VERSION}}"
CACHE_DIR="${AGENT_DOCTOR_EXO_DASHBOARD_CACHE:-${HOME}/.agent-doctor/cache/exo-dashboard/${PINNED_VERSION}}"
SRC_DIR="${CACHE_DIR}/src"
OVERLAY_DIR="${ROOT}/vendor/exo-dashboard-zh"
OUT_DIR="${CACHE_DIR}/dashboard-zh"
INSTALL_DIR="${HOME}/.agent-doctor/runtimes/exo/${PINNED_VERSION}/dashboard-zh"

DO_INSTALL=1
APPLY_ZH=1
for arg in "$@"; do
  case "$arg" in
    --no-install) DO_INSTALL=0 ;;
    --english-only) APPLY_ZH=0 ;;
    -h|--help)
      sed -n '2,14p' "$0"
      exit 0
      ;;
  esac
done

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "缺少命令：$1" >&2
    exit 1
  }
}

need git
need npm
need python3

mkdir -p "${CACHE_DIR}"
if [[ ! -d "${SRC_DIR}/.git" ]]; then
  echo "==> 拉取 exo@${EXO_REF}（仅 dashboard/）"
  rm -rf "${SRC_DIR}"
  mkdir -p "${SRC_DIR}"
  git -C "${SRC_DIR}" init -q
  git -C "${SRC_DIR}" remote add origin https://github.com/exo-explore/exo.git
  git -C "${SRC_DIR}" sparse-checkout init --cone
  git -C "${SRC_DIR}" sparse-checkout set dashboard
  if ! git -C "${SRC_DIR}" fetch --depth 1 origin "refs/tags/${EXO_REF}:refs/tags/${EXO_REF}"; then
    echo "标签 ${EXO_REF} 不存在，改拉 main（仅用于看板文案开发；正式包请锁到与运行时一致的 tag）" >&2
    git -C "${SRC_DIR}" fetch --depth 1 origin main
    git -C "${SRC_DIR}" checkout -q FETCH_HEAD
  else
    git -C "${SRC_DIR}" checkout -q "tags/${EXO_REF}"
  fi
else
  echo "==> 复用已有 checkout：${SRC_DIR}"
fi

DASHBOARD="${SRC_DIR}/dashboard"
if [[ ! -f "${DASHBOARD}/package.json" ]]; then
  echo "未找到 ${DASHBOARD}/package.json" >&2
  exit 1
fi

WORK="${CACHE_DIR}/work"
rm -rf "${WORK}"
mkdir -p "${WORK}"
# Prefer ditto/cp -R to keep binary assets.
if command -v ditto >/dev/null 2>&1; then
  ditto "${DASHBOARD}" "${WORK}"
else
  cp -R "${DASHBOARD}/." "${WORK}/"
fi

# Apache-2.0 §4(b): mark modified files when we apply zh overlays.
NOTICE="${WORK}/AGENT_DOCTOR_ZH_NOTICE.txt"
cat >"${NOTICE}" <<EOF
This dashboard build may include Chinese UI string overlays and UX trims prepared for Agent Doctor.
Upstream: https://github.com/exo-explore/exo (Apache-2.0)
Pinned ref: ${EXO_REF}
Overlay directory: vendor/exo-dashboard-zh/
UX: hide exo chat only (topology-only default); keep onboarding and all other pages;
adds Suitable models route for Agent Doctor.
EOF

BOARD_UI="${ROOT}/vendor/board-ui"
if [[ -d "${BOARD_UI}" ]]; then
  echo "==> 装入共用看板模块（vendor/board-ui）"
  mkdir -p "${WORK}/src/board-ui" "${WORK}/src/lib/board-ui"
  cp "${BOARD_UI}/theme.css" "${WORK}/src/board-ui/theme.css"
  cp "${BOARD_UI}/topology.ts" "${WORK}/src/lib/board-ui/topology.ts"
fi

if [[ -f "${OVERLAY_DIR}/agent-doctor-theme.css" ]]; then
  cp "${OVERLAY_DIR}/agent-doctor-theme.css" "${WORK}/src/agent-doctor-theme.css"
fi

if [[ -d "${OVERLAY_DIR}/components" ]]; then
  echo "==> 复制 Agent Doctor 看板组件覆盖（vendor/exo-dashboard-zh/components）"
  if command -v ditto >/dev/null 2>&1; then
    ditto "${OVERLAY_DIR}/components" "${WORK}/src/lib/components"
  else
    cp -R "${OVERLAY_DIR}/components/." "${WORK}/src/lib/components/"
  fi
fi

# Agent Doctor-only routes first, then UX trim, then Chinese strings —
# so overlay pages (e.g. suitable) also get TSV translation.
if [[ -d "${OVERLAY_DIR}/routes" ]]; then
  echo "==> 复制 Agent Doctor 看板路由覆盖（vendor/exo-dashboard-zh/routes）"
  if command -v ditto >/dev/null 2>&1; then
    ditto "${OVERLAY_DIR}/routes" "${WORK}/src/routes"
  else
    cp -R "${OVERLAY_DIR}/routes/." "${WORK}/src/routes/"
  fi
fi

# Hide exo chat only; keep onboarding / downloads / integrations / suitable.
if [[ -f "${OVERLAY_DIR}/apply_ux_patch.py" ]]; then
  echo "==> 应用 Agent Doctor 看板 UX 裁剪（只隐藏对话，保留引导与其他页）"
  python3 "${OVERLAY_DIR}/apply_ux_patch.py" "${WORK}"
fi

if [[ "${APPLY_ZH}" -eq 1 && -f "${OVERLAY_DIR}/strings.tsv" ]]; then
  echo "==> 应用中文文案覆盖（vendor/exo-dashboard-zh/strings.tsv）"
  python3 - <<'PY' "${WORK}" "${OVERLAY_DIR}/strings.tsv"
import pathlib, re, sys

work = pathlib.Path(sys.argv[1])
tsv = pathlib.Path(sys.argv[2])
pairs = []
for line in tsv.read_text(encoding="utf-8").splitlines():
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    if "\t" not in line:
        raise SystemExit(f"bad tsv line (need EN<TAB>ZH): {line}")
    en, zh = line.split("\t", 1)
    en = en.replace("\\n", "\n").replace("\\t", "\t")
    zh = zh.replace("\\n", "\n").replace("\\t", "\t")
    if en:
        pairs.append((en, zh))

# Only rewrite string literals and HTML/Svelte text nodes so identifiers like
# ChatForm / DownloadCompleted stay intact.
quote_re = re.compile(r"(['\"`])((?:\\.|(?!\1).)*)(\1)", re.DOTALL)
text_node_re = re.compile(r"(>)([^<>{]+)(<)")

def rewrite_chunk(chunk: str) -> str:
    """Replace user-facing copy without touching identifier fragments.

    - Phrases (contain whitespace) or long tokens (>= 12 chars): substring replace
    - Short tokens: only when the whole quoted/text-node content equals the EN
      string (ignoring surrounding whitespace)
    """
    stripped = chunk.strip()
    out = chunk
    for en, zh in pairs:
        if (" " in en) or ("…" in en) or ("—" in en) or (len(en) >= 12):
            if en in out:
                out = out.replace(en, zh)
        elif stripped == en:
            # Preserve original leading/trailing whitespace.
            lead = chunk[: len(chunk) - len(chunk.lstrip())]
            trail = chunk[len(chunk.rstrip()) :]
            out = f"{lead}{zh}{trail}"
    return out

changed = 0
for path in work.rglob("*"):
    if not path.is_file():
        continue
    if path.suffix not in {".svelte", ".ts", ".js", ".html", ".css", ".json"}:
        continue
    if "node_modules" in path.parts or path.name.endswith(".map"):
        continue
    text = path.read_text(encoding="utf-8")

    def quote_sub(m: re.Match[str]) -> str:
        return m.group(1) + rewrite_chunk(m.group(2)) + m.group(3)

    def text_sub(m: re.Match[str]) -> str:
        return m.group(1) + rewrite_chunk(m.group(2)) + m.group(3)

    new = quote_re.sub(quote_sub, text)
    new = text_node_re.sub(text_sub, new)
    # Phrases with whitespace are safe globally (cannot be identifiers) and catch
    # copy sitting inside Svelte {#if}/{#each} blocks that text_node_re misses.
    for en, zh in pairs:
        if (" " in en) or ("…" in en) or ("—" in en):
            if en in new:
                new = new.replace(en, zh)
    if new != text:
        path.write_text(new, encoding="utf-8")
        changed += 1
print(f"patched {changed} files with {len(pairs)} string pairs (literals/text only)")
PY
elif [[ "${APPLY_ZH}" -eq 1 ]]; then
  echo "==> 未找到 ${OVERLAY_DIR}/strings.tsv，先构建英文看板；补上 TSV 后再跑即可出中文包"
fi

echo "==> npm ci + build"
(
  cd "${WORK}"
  if [[ -f package-lock.json ]]; then
    npm ci --no-fund --no-audit
  else
    npm install --no-fund --no-audit
  fi
  npm run build
)

BUILD_OUT=""
for candidate in "${WORK}/build" "${WORK}/.svelte-kit/output/client" "${WORK}/dist"; do
  if [[ -f "${candidate}/index.html" ]]; then
    BUILD_OUT="${candidate}"
    break
  fi
done
if [[ -z "${BUILD_OUT}" ]]; then
  echo "找不到构建产物 index.html（检查 adapter-static 输出目录）" >&2
  find "${WORK}" -name index.html | head -20 >&2 || true
  exit 1
fi

rm -rf "${OUT_DIR}"
mkdir -p "${OUT_DIR}"
if command -v ditto >/dev/null 2>&1; then
  ditto "${BUILD_OUT}" "${OUT_DIR}"
else
  cp -R "${BUILD_OUT}/." "${OUT_DIR}/"
fi
cp "${NOTICE}" "${OUT_DIR}/AGENT_DOCTOR_ZH_NOTICE.txt"
echo "==> 构建完成：${OUT_DIR}"

if [[ "${DO_INSTALL}" -eq 1 ]]; then
  mkdir -p "$(dirname "${INSTALL_DIR}")"
  rm -rf "${INSTALL_DIR}"
  if command -v ditto >/dev/null 2>&1; then
    ditto "${OUT_DIR}" "${INSTALL_DIR}"
  else
    mkdir -p "${INSTALL_DIR}"
    cp -R "${OUT_DIR}/." "${INSTALL_DIR}/"
  fi
  echo "==> 已安装到 ${INSTALL_DIR}"
  echo "重启多机算力（或重新点开启）后，点「打开算力看板」即可在桌面内嵌窗口加载该静态页。"
fi
