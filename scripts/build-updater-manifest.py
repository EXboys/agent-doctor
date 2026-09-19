#!/usr/bin/env python3
"""Build Tauri updater latest.json from release artifacts.

Usage:
  python3 scripts/build-updater-manifest.py \\
    --artifacts ./desktop-artifacts \\
    --version 0.1.35 \\
    --cdn-base https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop \\
    --github-base https://github.com/EXboys/agent-doctor/releases/download/v0.1.35 \\
    --out-dir ./updater-manifest
"""

from __future__ import annotations

import argparse
import json
import re
from datetime import datetime, timezone
from pathlib import Path


# Prefer updater-friendly bundles (signed by createUpdaterArtifacts).
CANDIDATES: list[tuple[str, list[str]]] = [
    ("darwin-aarch64", [r"Agent Doctor\.app\.tar\.gz$", r".*\.app\.tar\.gz$"]),
    ("darwin-x86_64", [r"Agent Doctor\.app\.tar\.gz$", r".*\.app\.tar\.gz$"]),
    ("windows-x86_64", [r".*-setup\.exe$", r".*setup\.exe$", r".*\.nsis\.zip$"]),
    ("linux-x86_64", [r".*\.AppImage$"]),
]


def read_sig(path: Path) -> str:
    sig = path.with_name(path.name + ".sig")
    if not sig.is_file():
        raise SystemExit(f"missing signature: {sig}")
    return sig.read_text(encoding="utf-8").strip()


def find_files(root: Path) -> list[Path]:
    return [p for p in root.rglob("*") if p.is_file() and not p.name.endswith(".sig")]


def pick_for_platform(platform: str, files: list[Path], arch_hint: str | None) -> Path | None:
    patterns = next((pats for key, pats in CANDIDATES if key == platform), [])
    matched: list[Path] = []
    for path in files:
        name = path.name
        if arch_hint and arch_hint not in name.lower() and platform.startswith("darwin"):
            # macOS artifacts often lack arch in filename when built on a single runner.
            pass
        for pat in patterns:
            if re.search(pat, name, re.IGNORECASE):
                matched.append(path)
                break
    if not matched:
        return None
    # Prefer updater archives over installers when both exist.
    matched.sort(
        key=lambda p: (
            0 if p.name.endswith(".app.tar.gz") else 1,
            0 if "setup" in p.name.lower() else 1,
            len(p.name),
        )
    )
    return matched[0]


def build_manifest(
    *,
    version: str,
    notes: str,
    base_url: str,
    platforms: dict[str, Path],
) -> dict:
    base = base_url.rstrip("/")
    out: dict = {
        "version": version.lstrip("v"),
        "notes": notes,
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {},
    }
    for platform, path in platforms.items():
        out["platforms"][platform] = {
            "url": f"{base}/{path.name}",
            "signature": read_sig(path),
        }
    return out


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--cdn-base", required=True)
    parser.add_argument("--github-base", required=True)
    parser.add_argument("--notes", default="")
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args()

    files = find_files(args.artifacts)
    if not files:
        raise SystemExit(f"no artifacts under {args.artifacts}")

    # Current CI builds one macOS arch per job (arm64 on macos-14).
    # Map whatever .app.tar.gz we find to darwin-aarch64 by default;
    # if a filename contains x86_64 / aarch64, prefer that key.
    platforms: dict[str, Path] = {}
    app_tars = [p for p in files if p.name.endswith(".app.tar.gz")]
    for path in app_tars:
        lower = path.name.lower()
        if "x86_64" in lower or "x64" in lower or "amd64" in lower:
            platforms["darwin-x86_64"] = path
        else:
            platforms["darwin-aarch64"] = path

    win = pick_for_platform("windows-x86_64", files, None)
    if win:
        platforms["windows-x86_64"] = win
    linux = pick_for_platform("linux-x86_64", files, None)
    if linux:
        platforms["linux-x86_64"] = linux

    if not platforms:
        raise SystemExit("could not map any updater platforms from artifacts")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    notes = args.notes or f"Agent Doctor {args.version}"

    cdn_manifest = build_manifest(
        version=args.version,
        notes=notes,
        base_url=args.cdn_base,
        platforms=platforms,
    )
    gh_manifest = build_manifest(
        version=args.version,
        notes=notes,
        base_url=args.github_base,
        platforms=platforms,
    )

    (args.out_dir / "latest.json").write_text(
        json.dumps(cdn_manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    # GitHub endpoint fallback uses GitHub asset URLs (works when CDN is down overseas).
    (args.out_dir / "latest.github.json").write_text(
        json.dumps(gh_manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    # Also publish as latest.json on GitHub so the configured endpoint works;
    # prefer CDN URLs inside it so mainland users who somehow hit GitHub JSON
    # still download packages from CDN when available.
    (args.out_dir / "latest.release.json").write_text(
        json.dumps(cdn_manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )

    # Copy packages + signatures next to manifests for CDN sync.
    packages_dir = args.out_dir / "packages"
    packages_dir.mkdir(exist_ok=True)
    for path in platforms.values():
        target = packages_dir / path.name
        target.write_bytes(path.read_bytes())
        sig = path.with_name(path.name + ".sig")
        (packages_dir / sig.name).write_text(sig.read_text(encoding="utf-8"), encoding="utf-8")

    print(f"platforms: {', '.join(sorted(platforms))}")
    print(f"wrote manifests to {args.out_dir}")


if __name__ == "__main__":
    main()
