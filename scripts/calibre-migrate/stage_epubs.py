#!/usr/bin/env python3
"""Hardlink (or copy) EPUBs from the Calibre library into a flat staging dir.

Staging names are {calibre_id}.epub, matching epub_manifest.jsonl.
Prefer hardlinks when source and staging share a filesystem; fall back to copy.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from pathlib import Path


def load_manifest(path: Path) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def link_or_copy(src: Path, dest: Path) -> str:
    if dest.exists():
        if dest.stat().st_size == src.stat().st_size:
            return "exists"
        dest.unlink()
    try:
        os.link(src, dest)
        return "hardlink"
    except OSError:
        shutil.copy2(src, dest)
        return "copy"


def stage(manifest_path: Path, staging_dir: Path) -> int:
    if not manifest_path.is_file():
        print(f"error: manifest not found: {manifest_path}", file=sys.stderr)
        return 1

    rows = load_manifest(manifest_path)
    staging_dir.mkdir(parents=True, exist_ok=True)

    linked = copied = existed = missing = 0
    for row in rows:
        src = Path(row["source_epub"])
        dest = staging_dir / row["staging_name"]
        if not src.is_file():
            print(f"missing: {src}", file=sys.stderr)
            missing += 1
            continue
        how = link_or_copy(src, dest)
        if how == "hardlink":
            linked += 1
        elif how == "copy":
            copied += 1
        else:
            existed += 1

    print(f"manifest books: {len(rows)}")
    print(f"hardlinked:     {linked}")
    print(f"copied:         {copied}")
    print(f"already staged: {existed}")
    print(f"missing source: {missing}")
    print(f"staging dir:    {staging_dir}")
    return 1 if missing else 0


def main() -> int:
    here = Path(__file__).resolve().parent
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--manifest",
        type=Path,
        default=here / "out" / "epub_manifest.jsonl",
    )
    p.add_argument(
        "--staging",
        type=Path,
        default=here / "staging",
    )
    args = p.parse_args()
    return stage(args.manifest.resolve(), args.staging.resolve())


if __name__ == "__main__":
    raise SystemExit(main())
