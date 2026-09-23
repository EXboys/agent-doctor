#!/usr/bin/env python3
"""Build TeamUps-facing download.json from installer files on disk.

Usage:
  python3 scripts/build-download-manifest.py \\
    --installers ./oss-installers \\
    --version 0.1.44 \\
    --cdn-base https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop \\
    --out ./download.json \\
    --edition personal
"""

from __future__ import annotations

import argparse
import json
from datetime import datetime, timezone
from pathlib import Path


def publish_name(path: Path) -> str:
    return path.name.replace(" ", ".")


def detect_platform(name: str, edition: str) -> str | None:
    n = name.lower()
    if edition == "team":
        if "team" not in n:
            return None
    else:
        if "team" in n:
            return None
    if n.endswith(".dmg"):
        if "aarch64" in n or "arm64" in n:
            return "macos-arm64"
        if "x64" in n or "x86_64" in n:
            return "macos-x64"
        return "macos-arm64"
    if n.endswith("-setup.exe") or n.endswith("setup.exe"):
        return "windows-x64"
    return None


def build(
    *,
    installers_dir: Path,
    version: str,
    cdn_base: str,
    edition: str,
    notes: str | None,
) -> dict:
    ver = version.lstrip("v")
    base = cdn_base.rstrip("/")
    assets: list[dict] = []
    for path in sorted(installers_dir.rglob("*")):
        if not path.is_file():
            continue
        name = publish_name(path)
        platform = detect_platform(name, edition)
        if not platform:
            continue
        assets.append(
            {
                "platform": platform,
                "fileName": name,
                "url": f"{base}/{name}",
                "sha256": "pending",
                "sizeBytes": path.stat().st_size,
            }
        )
    if not assets:
        raise SystemExit(f"no installers found under {installers_dir}")
    # stable order: windows, macos-arm, macos-x64
    order = {"windows-x64": 0, "macos-arm64": 1, "macos-x64": 2}
    assets.sort(key=lambda a: (order.get(a["platform"], 9), a["fileName"]))
    return {
        "version": ver,
        "notes": (notes or f"Agent Doctor v{ver}").strip(),
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "assets": assets,
    }


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--installers", type=Path, required=True)
    p.add_argument("--version", required=True)
    p.add_argument("--cdn-base", required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--edition", choices=("personal", "team"), default="personal")
    p.add_argument("--notes", default="")
    args = p.parse_args()
    data = build(
        installers_dir=args.installers,
        version=args.version,
        cdn_base=args.cdn_base,
        edition=args.edition,
        notes=args.notes or None,
    )
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
    print(f"wrote {args.out} ({len(data['assets'])} assets)")


if __name__ == "__main__":
    main()
