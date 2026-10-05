#!/usr/bin/env python3
"""Pick stable installer objects on OSS to delete, keeping the newest N versions.

Reads `ossutil ls` output on stdin and prints one oss:// URL per line to delete.
Only looks at objects directly under the prefix (beta/ and other subdirs are
left alone) whose name carries a stable X.Y.Z version, e.g.
Agent.Doctor_0.1.51_aarch64.dmg or Agent.Doctor_0.1.51_x64-setup.exe.sig.
Manifests (latest.json, download.json) and unversioned files are never listed.

Usage:
  ossutil ls oss://bucket/desktop/ | \\
    python3 scripts/prune-oss-installers.py --prefix oss://bucket/desktop/ --keep 3
"""

from __future__ import annotations

import argparse
import re
import sys

VERSION_RE = re.compile(r"_(\d+)\.(\d+)\.(\d+)_")
URL_RE = re.compile(r"oss://\S+")


def pick_deletions(lines: list[str], prefix: str, keep: int) -> list[str]:
    prefix = prefix.rstrip("/") + "/"
    by_version: dict[tuple[int, int, int], list[str]] = {}
    for line in lines:
        for url in URL_RE.findall(line):
            if not url.startswith(prefix):
                continue
            name = url[len(prefix) :]
            if not name or "/" in name:
                continue
            m = VERSION_RE.search(name)
            if not m:
                continue
            ver = (int(m.group(1)), int(m.group(2)), int(m.group(3)))
            by_version.setdefault(ver, []).append(url)
    stale = sorted(by_version, reverse=True)[keep:]
    return sorted(url for ver in stale for url in by_version[ver])


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--prefix", required=True, help="oss://bucket/desktop/")
    p.add_argument("--keep", type=int, default=3)
    args = p.parse_args()
    if args.keep < 1:
        raise SystemExit("--keep must be at least 1")
    for url in pick_deletions(sys.stdin.read().splitlines(), args.prefix, args.keep):
        print(url)


if __name__ == "__main__":
    main()
