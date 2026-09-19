#!/usr/bin/env python3
"""Build Tauri updater latest.json from release artifacts.

Usage:
  python3 scripts/build-updater-manifest.py \\
    --artifacts ./desktop-artifacts \\
    --version 0.1.39 \\
    --cdn-base https://agent-doctor.oss-cn-shenzhen.aliyuncs.com/desktop \\
    --github-base https://github.com/EXboys/agent-doctor/releases/download/v0.1.39 \\
    --out-dir ./updater-manifest
"""

from __future__ import annotations

import argparse
import json
import re
import urllib.parse
from datetime import datetime, timezone
from pathlib import Path


# Prefer updater-friendly bundles (signed by createUpdaterArtifacts).
CANDIDATES: list[tuple[str, list[str]]] = [
    ("darwin-aarch64", [r".*\.app\.tar\.gz$"]),
    ("darwin-x86_64", [r".*\.app\.tar\.gz$"]),
    ("windows-x86_64", [r".*-setup\.exe$", r".*setup\.exe$", r".*\.nsis\.zip$"]),
    ("linux-x86_64", [r".*\.AppImage$"]),
]


def publish_name(path: Path) -> str:
    """GitHub Releases rewrites spaces to dots; keep OSS / URLs consistent."""
    return path.name.replace(" ", ".")


def find_sig(bundle: Path, root: Path) -> Path | None:
    direct = bundle.with_name(bundle.name + ".sig")
    if direct.is_file():
        return direct
    # Signatures may land in a sibling artifact folder after merge-multiple.
    wanted = {bundle.name + ".sig", publish_name(bundle) + ".sig"}
    for sig in root.rglob("*.sig"):
        if sig.name in wanted:
            return sig
    return None


def find_files(root: Path) -> list[Path]:
    return [p for p in root.rglob("*") if p.is_file() and not p.name.endswith(".sig")]


def pick_for_platform(platform: str, files: list[Path]) -> Path | None:
    patterns = next((pats for key, pats in CANDIDATES if key == platform), [])
    matched: list[Path] = []
    for path in files:
        for pat in patterns:
            if re.search(pat, path.name, re.IGNORECASE):
                matched.append(path)
                break
    if not matched:
        return None
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
    platforms: dict[str, tuple[Path, Path]],
    urlencode_names: bool,
) -> dict:
    base = base_url.rstrip("/")
    out: dict = {
        "version": version.lstrip("v"),
        "notes": notes,
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {},
    }
    for platform, (bundle, sig) in platforms.items():
        name = publish_name(bundle)
        url_name = urllib.parse.quote(name) if urlencode_names else name
        out["platforms"][platform] = {
            "url": f"{base}/{url_name}",
            "signature": sig.read_text(encoding="utf-8").strip(),
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

    candidates: dict[str, Path] = {}
    app_tars = [p for p in files if p.name.endswith(".app.tar.gz")]
    for path in app_tars:
        lower = path.name.lower()
        if "x86_64" in lower or "x64" in lower or "amd64" in lower:
            candidates["darwin-x86_64"] = path
        else:
            candidates["darwin-aarch64"] = path

    win = pick_for_platform("windows-x86_64", files)
    if win:
        candidates["windows-x86_64"] = win
    linux = pick_for_platform("linux-x86_64", files)
    if linux:
        candidates["linux-x86_64"] = linux

    platforms: dict[str, tuple[Path, Path]] = {}
    skipped: list[str] = []
    for platform, bundle in candidates.items():
        sig = find_sig(bundle, args.artifacts)
        if not sig:
            skipped.append(f"{platform} ({bundle.name})")
            continue
        platforms[platform] = (bundle, sig)

    if skipped:
        print("skipping platforms without .sig: " + ", ".join(skipped))
    if not platforms:
        raise SystemExit("could not map any signed updater platforms from artifacts")

    args.out_dir.mkdir(parents=True, exist_ok=True)
    notes = args.notes or f"Agent Doctor {args.version}"

    cdn_manifest = build_manifest(
        version=args.version,
        notes=notes,
        base_url=args.cdn_base,
        platforms=platforms,
        urlencode_names=False,
    )
    gh_manifest = build_manifest(
        version=args.version,
        notes=notes,
        base_url=args.github_base,
        platforms=platforms,
        # GitHub asset URLs use the sanitized filename; encoding spaces is safer.
        urlencode_names=True,
    )

    (args.out_dir / "latest.json").write_text(
        json.dumps(cdn_manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    (args.out_dir / "latest.github.json").write_text(
        json.dumps(gh_manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )

    packages_dir = args.out_dir / "packages"
    packages_dir.mkdir(exist_ok=True)
    for bundle, sig in platforms.values():
        out_bundle = packages_dir / publish_name(bundle)
        out_sig = packages_dir / (publish_name(bundle) + ".sig")
        out_bundle.write_bytes(bundle.read_bytes())
        out_sig.write_text(sig.read_text(encoding="utf-8"), encoding="utf-8")

    print(f"platforms: {', '.join(sorted(platforms))}")
    print(f"wrote manifests to {args.out_dir}")


if __name__ == "__main__":
    main()
