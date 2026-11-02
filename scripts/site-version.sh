#!/usr/bin/env bash
# Stamp the static assets of the marketing site with a content hash.
#
# Cloudflare Pages serves /css and /js with a four hour edge cache and
# ignores the Cache-Control in _headers, so a plain deploy can hand a reader
# a new HTML file next to the stylesheet from the previous deploy. The ?v=
# stamp gives every content change a new cache key, so the edge always serves
# the file that matches the HTML.
#
# Run this after changing site/css/site.css or any site/js/*.js file, and
# before deploying the site.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT/site"

hash="$(cat css/site.css js/nav.js js/site.js js/skills.js | shasum -a 256 | cut -c1-8)"

for page in index.html skills.html 404.html; do
  perl -pi -e "s{(/(?:css/site\\.css|js/(?:nav|site|skills)\\.js))(?:\\?v=[0-9a-f]+)?}{\$1?v=$hash}g" "$page"
done

echo "site assets stamped $hash"
