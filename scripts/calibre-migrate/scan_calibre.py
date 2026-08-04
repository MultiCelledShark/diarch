#!/usr/bin/env python3
"""Scan a Calibre library and classify books for Diarch EPUB migration.

Writes:
  out/epub_manifest.jsonl  — one line per book that has an EPUB (import candidates)
  out/needs_conversion.csv — books with no EPUB (triage for Calibre convert / hand work)

Counts are per Calibre book (books.id), not per format file.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import sqlite3
import sys
from pathlib import Path

EASY_CONVERT = frozenset({"MOBI", "AZW3", "DOCX", "TXT", "HTMLZ", "AZW", "POBI"})
HAND_HINT = frozenset({"PDF", "KFX", "DJVU", "CBZ", "ZIP", "CBR"})


def connect_ro(db_path: Path) -> sqlite3.Connection:
    uri = f"file:{db_path}?mode=ro"
    return sqlite3.connect(uri, uri=True)


def author_names(con: sqlite3.Connection) -> dict[int, str]:
    ordered: dict[int, list[str]] = {}
    for book_id, name in con.execute(
        """
        SELECT bal.book, a.name
        FROM books_authors_link bal
        JOIN authors a ON a.id = bal.author
        ORDER BY bal.book, bal.id
        """
    ):
        ordered.setdefault(book_id, []).append(name)
    return {bid: " & ".join(names) for bid, names in ordered.items()}


def isbn_map(con: sqlite3.Connection) -> dict[int, str]:
    out: dict[int, str] = {}
    for book_id, val in con.execute(
        """
        SELECT book, val FROM identifiers
        WHERE lower(type) IN ('isbn', 'isbn13', 'isbn10')
        ORDER BY book, id
        """
    ):
        out.setdefault(book_id, val)
    return out


def suggest_action(formats: list[str]) -> str:
    upper = {f.upper() for f in formats}
    if upper & EASY_CONVERT:
        return "calibre_convert"
    return "hand_review"


def scan(library: Path, out_dir: Path) -> int:
    db_path = library / "metadata.db"
    if not db_path.is_file():
        print(f"error: metadata.db not found at {db_path}", file=sys.stderr)
        return 1

    out_dir.mkdir(parents=True, exist_ok=True)
    manifest_path = out_dir / "epub_manifest.jsonl"
    convert_path = out_dir / "needs_conversion.csv"

    con = connect_ro(db_path)
    authors = author_names(con)
    isbns = isbn_map(con)

    books = con.execute(
        """
        SELECT id, uuid, title, path, isbn
        FROM books
        ORDER BY id
        """
    ).fetchall()

    formats_by_book: dict[int, list[tuple[str, str]]] = {}
    for book_id, fmt, name in con.execute(
        "SELECT book, format, name FROM data ORDER BY book, format"
    ):
        formats_by_book.setdefault(book_id, []).append((fmt.upper(), name))

    epub_count = 0
    epub_bytes = 0
    convert_count = 0
    missing_epub_file = 0
    suggested_counts: dict[str, int] = {}

    with manifest_path.open("w", encoding="utf-8") as mf, convert_path.open(
        "w", encoding="utf-8", newline=""
    ) as cf:
        writer = csv.DictWriter(
            cf,
            fieldnames=[
                "calibre_id",
                "uuid",
                "title",
                "authors",
                "path",
                "formats",
                "has_pdf",
                "suggested",
            ],
        )
        writer.writeheader()

        for book_id, uuid, title, rel_path, books_isbn in books:
            fmts = formats_by_book.get(book_id, [])
            format_names = [f for f, _ in fmts]
            author_str = authors.get(book_id, "")
            isbn = isbns.get(book_id) or (books_isbn or None)
            if isbn is not None:
                isbn = str(isbn).strip() or None

            epub_name = next((name for fmt, name in fmts if fmt == "EPUB"), None)
            if epub_name is not None:
                epub_path = library / rel_path / f"{epub_name}.epub"
                if not epub_path.is_file():
                    missing_epub_file += 1
                    suggested = "missing_epub_file"
                    writer.writerow(
                        {
                            "calibre_id": book_id,
                            "uuid": uuid,
                            "title": title,
                            "authors": author_str,
                            "path": rel_path,
                            "formats": ",".join(format_names),
                            "has_pdf": "1" if "PDF" in format_names else "0",
                            "suggested": suggested,
                        }
                    )
                    suggested_counts[suggested] = suggested_counts.get(suggested, 0) + 1
                    convert_count += 1
                    continue

                size = epub_path.stat().st_size
                staging_name = f"{book_id}.epub"
                record = {
                    "calibre_id": book_id,
                    "calibre_uuid": uuid,
                    "title": title,
                    "authors": author_str,
                    "isbn": isbn,
                    "calibre_path": rel_path,
                    "source_epub": str(epub_path),
                    "staging_name": staging_name,
                    "bytes": size,
                    "formats": format_names,
                }
                mf.write(json.dumps(record, ensure_ascii=False) + "\n")
                epub_count += 1
                epub_bytes += size
            else:
                suggested = suggest_action(format_names)
                writer.writerow(
                    {
                        "calibre_id": book_id,
                        "uuid": uuid,
                        "title": title,
                        "authors": author_str,
                        "path": rel_path,
                        "formats": ",".join(format_names),
                        "has_pdf": "1" if "PDF" in format_names else "0",
                        "suggested": suggested,
                    }
                )
                suggested_counts[suggested] = suggested_counts.get(suggested, 0) + 1
                convert_count += 1

    con.close()

    print(f"library:            {library}")
    print(f"books scanned:      {len(books)}")
    print(f"epub candidates:    {epub_count} ({epub_bytes / 1e9:.2f} GB)")
    print(f"needs conversion:   {convert_count}")
    for key in sorted(suggested_counts):
        print(f"  {key}: {suggested_counts[key]}")
    if missing_epub_file:
        print(f"missing epub file:  {missing_epub_file} (listed in needs_conversion)")
    print(f"wrote:              {manifest_path}")
    print(f"wrote:              {convert_path}")
    return 0


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--library",
        type=Path,
        default=Path(os.environ.get("CALIBRE_LIBRARY", "/home/igrot/Library")),
        help="Calibre library root (contains metadata.db)",
    )
    p.add_argument(
        "--out",
        type=Path,
        default=Path(__file__).resolve().parent / "out",
        help="Output directory for manifest + CSV",
    )
    args = p.parse_args()
    return scan(args.library.resolve(), args.out.resolve())


if __name__ == "__main__":
    raise SystemExit(main())
