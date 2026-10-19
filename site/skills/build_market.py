#!/usr/bin/env python3
"""Zip unmodified upstream skills and write catalog.json plus manifests."""

import hashlib
import json
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SRC = ROOT / "src"


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def zip_dir(skill_id: str, folder: Path) -> bytes:
    dest = ROOT / skill_id / f"{skill_id}.zip"
    dest.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(folder.rglob("*")):
            if not path.is_file():
                continue
            archive.write(path, arcname=f"{skill_id}/{path.relative_to(folder).as_posix()}")
    return dest.read_bytes()


def manifest(skill_id: str, version: str, digest: str) -> dict:
    return {
        "slug": skill_id,
        "version": version,
        "skills": [{"id": skill_id, "version": version, "sha256": digest}],
        "artifact": {
            "url": f"/skills/{skill_id}/{skill_id}.zip",
            "version": version,
            "sha256": digest,
        },
    }


def main() -> None:
    market = json.loads((ROOT / "market.json").read_text(encoding="utf-8"))
    version = market["version"]
    items = []
    packs = []
    for skill in market["skills"]:
        skill_id = skill["id"]
        folder = SRC / skill_id
        skill_md = folder / "SKILL.md"
        if not skill_md.is_file():
            raise SystemExit(f"missing upstream skill: {skill_md}")
        data = zip_dir(skill_id, folder)
        digest = sha256(data)
        (ROOT / skill_id / "manifest.json").write_text(
            json.dumps(manifest(skill_id, version, digest), indent=2) + "\n",
            encoding="utf-8",
        )
        items.append(
            {
                **skill,
                "version": version,
                "free": True,
                "download": f"/skills/{skill_id}/{skill_id}.zip",
                "sha256": digest,
            }
        )
        packs.append(
            {
                "slug": skill_id,
                "id": skill_id,
                "kind": "skill",
                "name": skill["name"],
                "title": skill["name"],
                "description": skill["summary"],
                "free": True,
                "priceCents": 0,
                "version": version,
                "skill_count": 1,
                "license": skill["license"],
                "source": skill["source"],
                "skills": [{"id": skill_id, "version": version, "sha256": digest}],
            }
        )
    bundle_dir = ROOT / "everyday"
    bundle_dir.mkdir(exist_ok=True)
    bundle_path = bundle_dir / "everyday.zip"
    with zipfile.ZipFile(bundle_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for skill in market["skills"]:
            folder = SRC / skill["id"]
            for path in sorted(folder.rglob("*")):
                if path.is_file():
                    archive.write(
                        path,
                        arcname=f"{skill['id']}/{path.relative_to(folder).as_posix()}",
                    )
    bundle_bytes = bundle_path.read_bytes()
    catalog = {
        "version": version,
        "bundle": {
            "id": "everyday",
            "name": "All five",
            "zh": "五个一起下",
            "summary": "The five open source skills in one zip.",
            "download": "/skills/everyday/everyday.zip",
            "sha256": sha256(bundle_bytes),
            "version": version,
        },
        "items": items,
        "packs": packs,
    }
    (ROOT / "catalog.json").write_text(
        json.dumps(catalog, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"built {len(items)} upstream skills")


if __name__ == "__main__":
    main()
