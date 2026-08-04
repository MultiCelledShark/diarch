#!/usr/bin/env python3
"""Import staged Calibre EPUBs into Diarch via the local HTTP API.

Resume-safe: tracks each Calibre book id in import_ledger.sqlite.
Intended to run on keystone against http://127.0.0.1:8083 after rsync.
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import sqlite3
import sys
import threading
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


class DiarchClient:
    def __init__(self, base_url: str, token: str | None = None):
        self.base_url = base_url.rstrip("/")
        self.token = token

    def login(self, username: str, password: str) -> str:
        body = json.dumps({"username": username, "password": password}).encode()
        req = urllib.request.Request(
            f"{self.base_url}/api/auth/login",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=60) as resp:
            data = json.load(resp)
        token = data.get("token")
        if not token:
            raise RuntimeError(f"login response missing token: {data}")
        self.token = token
        return token

    def _headers(self) -> dict[str, str]:
        if not self.token:
            raise RuntimeError("not logged in")
        return {"Authorization": f"Bearer {self.token}"}

    def get_json(self, path: str) -> Any:
        req = urllib.request.Request(
            f"{self.base_url}{path}",
            headers=self._headers(),
            method="GET",
        )
        with urllib.request.urlopen(req, timeout=120) as resp:
            return json.load(resp)

    def work_exists(self, work_id: str) -> bool:
        try:
            self.get_json(f"/api/works/{work_id}")
            return True
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return False
            raise

    def get_job(self, job_id: str) -> dict:
        return self.get_json(f"/api/jobs/{job_id}")

    def import_epub(
        self,
        epub_path: Path,
        title: str,
        authors: str,
        filename: str | None = None,
    ) -> dict:
        """Multipart POST /api/library/import (stdlib only)."""
        boundary = f"----diarch{os.urandom(8).hex()}"
        filename = filename or epub_path.name
        file_bytes = epub_path.read_bytes()

        parts: list[bytes] = []

        def add_field(name: str, value: str) -> None:
            parts.append(
                (
                    f"--{boundary}\r\n"
                    f'Content-Disposition: form-data; name="{name}"\r\n\r\n'
                    f"{value}\r\n"
                ).encode()
            )

        add_field("title", title)
        if authors:
            add_field("authors", authors)

        parts.append(
            (
                f"--{boundary}\r\n"
                f'Content-Disposition: form-data; name="file"; filename="{filename}"\r\n'
                f"Content-Type: application/epub+zip\r\n\r\n"
            ).encode()
        )
        parts.append(file_bytes)
        parts.append(b"\r\n")
        parts.append(f"--{boundary}--\r\n".encode())
        body = b"".join(parts)

        req = urllib.request.Request(
            f"{self.base_url}/api/library/import",
            data=body,
            headers={
                **self._headers(),
                "Content-Type": f"multipart/form-data; boundary={boundary}",
                "Content-Length": str(len(body)),
            },
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=600) as resp:
                return json.load(resp)
        except urllib.error.HTTPError as e:
            detail = e.read().decode("utf-8", errors="replace")
            raise RuntimeError(f"import HTTP {e.code}: {detail}") from e


def open_ledger(path: Path) -> tuple[sqlite3.Connection, threading.Lock]:
    con = sqlite3.connect(path, check_same_thread=False)
    con.row_factory = sqlite3.Row
    con.execute(
        """
        CREATE TABLE IF NOT EXISTS imports (
            calibre_id INTEGER PRIMARY KEY,
            calibre_uuid TEXT,
            title TEXT,
            authors TEXT,
            source_epub TEXT,
            work_id TEXT,
            job_id TEXT,
            status TEXT NOT NULL,
            error TEXT,
            updated_at TEXT
        )
        """
    )
    con.commit()
    return con, threading.Lock()


def upsert(
    con: sqlite3.Connection,
    lock: threading.Lock,
    *,
    calibre_id: int,
    calibre_uuid: str | None,
    title: str,
    authors: str,
    source_epub: str,
    work_id: str | None = None,
    job_id: str | None = None,
    status: str,
    error: str | None = None,
) -> None:
    with lock:
        con.execute(
            """
            INSERT INTO imports (
                calibre_id, calibre_uuid, title, authors, source_epub,
                work_id, job_id, status, error, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(calibre_id) DO UPDATE SET
                calibre_uuid=excluded.calibre_uuid,
                title=excluded.title,
                authors=excluded.authors,
                source_epub=excluded.source_epub,
                work_id=COALESCE(excluded.work_id, imports.work_id),
                job_id=COALESCE(excluded.job_id, imports.job_id),
                status=excluded.status,
                error=excluded.error,
                updated_at=excluded.updated_at
            """,
            (
                calibre_id,
                calibre_uuid,
                title,
                authors,
                source_epub,
                work_id,
                job_id,
                status,
                error,
                utc_now(),
            ),
        )
        con.commit()


def load_manifest(path: Path) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def should_skip(con: sqlite3.Connection, client: DiarchClient, calibre_id: int) -> str | None:
    row = con.execute(
        "SELECT status, work_id, job_id FROM imports WHERE calibre_id = ?",
        (calibre_id,),
    ).fetchone()
    if row is None:
        return None
    status, work_id, job_id = row["status"], row["work_id"], row["job_id"]
    if status == "ok":
        return "ok"
    if status == "uploaded" and work_id and job_id:
        # Resume: poll existing job instead of re-uploading.
        return "poll"
    if status == "uploaded" and work_id:
        if client.work_exists(work_id):
            return "ok"
    return None


def poll_job(
    client: DiarchClient,
    job_id: str,
    *,
    timeout_s: float,
    interval_s: float,
) -> tuple[str, str | None]:
    deadline = time.monotonic() + timeout_s
    while True:
        job = client.get_job(job_id)
        # API may return the job object directly or wrapped.
        if isinstance(job, dict) and "job" in job:
            job = job["job"]
        status = (job.get("status") or "").lower()
        detail = job.get("detail")
        if status in ("done", "ok"):
            return "ok", None
        if status == "failed":
            return "failed", str(detail) if detail else "job failed"
        if time.monotonic() >= deadline:
            return "failed", f"job poll timeout after {timeout_s:.0f}s (last={status})"
        time.sleep(interval_s)


def process_one(
    client: DiarchClient,
    staging_dir: Path,
    row: dict,
    *,
    poll_timeout: float,
    poll_interval: float,
    resume_job_id: str | None = None,
    resume_work_id: str | None = None,
) -> dict:
    calibre_id = int(row["calibre_id"])
    title = row.get("title") or f"Calibre {calibre_id}"
    authors = row.get("authors") or ""
    staging_name = row["staging_name"]
    epub_path = staging_dir / staging_name
    if not epub_path.is_file():
        return {
            "calibre_id": calibre_id,
            "status": "failed",
            "error": f"missing staged file: {epub_path}",
            "work_id": None,
            "job_id": None,
        }

    work_id = resume_work_id
    job_id = resume_job_id

    if job_id is None:
        resp = client.import_epub(epub_path, title=title, authors=authors, filename=staging_name)
        work = resp.get("work") or {}
        work_id = str(work.get("id") or resp.get("work_id") or "")
        job_id = str(resp.get("job_id") or "")
        if not work_id or not job_id:
            return {
                "calibre_id": calibre_id,
                "status": "failed",
                "error": f"unexpected import response: {resp}",
                "work_id": work_id or None,
                "job_id": job_id or None,
            }

    final, err = poll_job(
        client, job_id, timeout_s=poll_timeout, interval_s=poll_interval
    )
    return {
        "calibre_id": calibre_id,
        "calibre_uuid": row.get("calibre_uuid"),
        "title": title,
        "authors": authors,
        "source_epub": str(epub_path),
        "status": final,
        "error": err,
        "work_id": work_id,
        "job_id": job_id,
    }


def write_failures(path: Path, con: sqlite3.Connection) -> int:
    rows = con.execute(
        """
        SELECT calibre_id, calibre_uuid, title, authors, source_epub, work_id, job_id, error
        FROM imports WHERE status = 'failed'
        ORDER BY calibre_id
        """
    ).fetchall()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as f:
        w = csv.DictWriter(
            f,
            fieldnames=[
                "calibre_id",
                "calibre_uuid",
                "title",
                "authors",
                "source_epub",
                "work_id",
                "job_id",
                "error",
            ],
        )
        w.writeheader()
        for r in rows:
            w.writerow(dict(r))
    return len(rows)


def run(args: argparse.Namespace) -> int:
    manifest = args.manifest.resolve()
    staging = args.staging.resolve()
    ledger_path = args.ledger.resolve()
    out_dir = args.out.resolve()

    if not manifest.is_file():
        print(f"error: manifest not found: {manifest}", file=sys.stderr)
        return 1
    if not staging.is_dir():
        print(f"error: staging dir not found: {staging}", file=sys.stderr)
        return 1

    username = args.user or os.environ.get("DIARCH_ADMIN_USER") or os.environ.get("DIARCH_USER")
    password = args.password or os.environ.get("DIARCH_ADMIN_PASS") or os.environ.get("DIARCH_PASS")
    if not username or not password:
        print(
            "error: set --user/--password or DIARCH_ADMIN_USER / DIARCH_ADMIN_PASS",
            file=sys.stderr,
        )
        return 1

    client = DiarchClient(args.base_url)
    print(f"login → {args.base_url} as {username}")
    client.login(username, password)

    rows = load_manifest(manifest)
    con, ledger_lock = open_ledger(ledger_path)

    to_upload: list[dict] = []
    to_poll: list[tuple[dict, str, str]] = []  # row, work_id, job_id
    skipped_ok = 0
    limit = args.limit if args.limit and args.limit > 0 else None
    queued = 0

    for row in rows:
        cid = int(row["calibre_id"])
        action = should_skip(con, client, cid)
        if action == "ok":
            skipped_ok += 1
            # Normalize status if we discovered an existing work.
            existing = con.execute(
                "SELECT work_id, job_id, status FROM imports WHERE calibre_id=?",
                (cid,),
            ).fetchone()
            if existing and existing["status"] != "ok":
                upsert(
                    con,
                    ledger_lock,
                    calibre_id=cid,
                    calibre_uuid=row.get("calibre_uuid"),
                    title=row.get("title") or "",
                    authors=row.get("authors") or "",
                    source_epub=str(staging / row["staging_name"]),
                    work_id=existing["work_id"],
                    job_id=existing["job_id"],
                    status="ok",
                )
            continue
        if limit is not None and queued >= limit:
            continue
        if action == "poll":
            existing = con.execute(
                "SELECT work_id, job_id FROM imports WHERE calibre_id=?",
                (cid,),
            ).fetchone()
            to_poll.append((row, existing["work_id"], existing["job_id"]))
            queued += 1
            continue
        to_upload.append(row)
        queued += 1

    print(
        f"manifest={len(rows)} skip_ok={skipped_ok} "
        f"resume_poll={len(to_poll)} upload={len(to_upload)} "
        f"concurrency={args.concurrency}"
    )

    def handle_result(result: dict) -> None:
        upsert(
            con,
            ledger_lock,
            calibre_id=int(result["calibre_id"]),
            calibre_uuid=result.get("calibre_uuid"),
            title=result.get("title") or "",
            authors=result.get("authors") or "",
            source_epub=result.get("source_epub") or "",
            work_id=result.get("work_id"),
            job_id=result.get("job_id"),
            status=result["status"],
            error=result.get("error"),
        )
        mark = "OK" if result["status"] == "ok" else "FAIL"
        print(
            f"[{mark}] calibre_id={result['calibre_id']} "
            f"work={result.get('work_id')} job={result.get('job_id')} "
            f"{result.get('error') or ''}".rstrip()
        )

    # Mark uploads as uploaded immediately after POST inside worker via two-phase:
    # process_one does upload+poll; we also record uploaded mid-flight by
    # splitting for resume safety on interrupt — use a thin wrapper.
    def upload_and_poll(row: dict) -> dict:
        calibre_id = int(row["calibre_id"])
        title = row.get("title") or f"Calibre {calibre_id}"
        authors = row.get("authors") or ""
        epub_path = staging / row["staging_name"]
        if not epub_path.is_file():
            return {
                "calibre_id": calibre_id,
                "calibre_uuid": row.get("calibre_uuid"),
                "title": title,
                "authors": authors,
                "source_epub": str(epub_path),
                "status": "failed",
                "error": f"missing staged file: {epub_path}",
                "work_id": None,
                "job_id": None,
            }
        try:
            resp = client.import_epub(
                epub_path, title=title, authors=authors, filename=row["staging_name"]
            )
        except Exception as e:  # noqa: BLE001 — record and continue
            return {
                "calibre_id": calibre_id,
                "calibre_uuid": row.get("calibre_uuid"),
                "title": title,
                "authors": authors,
                "source_epub": str(epub_path),
                "status": "failed",
                "error": str(e),
                "work_id": None,
                "job_id": None,
            }

        work = resp.get("work") or {}
        work_id = str(work.get("id") or "")
        job_id = str(resp.get("job_id") or "")
        upsert(
            con,
            ledger_lock,
            calibre_id=calibre_id,
            calibre_uuid=row.get("calibre_uuid"),
            title=title,
            authors=authors,
            source_epub=str(epub_path),
            work_id=work_id or None,
            job_id=job_id or None,
            status="uploaded",
        )
        if not work_id or not job_id:
            return {
                "calibre_id": calibre_id,
                "calibre_uuid": row.get("calibre_uuid"),
                "title": title,
                "authors": authors,
                "source_epub": str(epub_path),
                "status": "failed",
                "error": f"unexpected import response: {resp}",
                "work_id": work_id or None,
                "job_id": job_id or None,
            }
        final, err = poll_job(
            client,
            job_id,
            timeout_s=args.poll_timeout,
            interval_s=args.poll_interval,
        )
        return {
            "calibre_id": calibre_id,
            "calibre_uuid": row.get("calibre_uuid"),
            "title": title,
            "authors": authors,
            "source_epub": str(epub_path),
            "status": final,
            "error": err,
            "work_id": work_id,
            "job_id": job_id,
        }

    work_items: list[tuple[str, Any]] = [("upload", r) for r in to_upload] + [
        ("poll", (r, wid, jid)) for r, wid, jid in to_poll
    ]

    ok = fail = 0
    with ThreadPoolExecutor(max_workers=max(1, args.concurrency)) as pool:
        futures = []
        for kind, payload in work_items:
            if kind == "upload":
                futures.append(pool.submit(upload_and_poll, payload))
            else:
                row, wid, jid = payload
                futures.append(
                    pool.submit(
                        process_one,
                        client,
                        staging,
                        row,
                        poll_timeout=args.poll_timeout,
                        poll_interval=args.poll_interval,
                        resume_job_id=jid,
                        resume_work_id=wid,
                    )
                )
        for fut in as_completed(futures):
            result = fut.result()
            handle_result(result)
            if result["status"] == "ok":
                ok += 1
            else:
                fail += 1

    failures_path = out_dir / "import_failures.csv"
    n_fail = write_failures(failures_path, con)

    summary = con.execute(
        "SELECT status, COUNT(*) FROM imports GROUP BY status ORDER BY status"
    ).fetchall()
    print("--- ledger summary ---")
    for status, count in summary:
        print(f"  {status}: {count}")
    print(f"this run: ok={ok} failed={fail} skipped_ok={skipped_ok}")
    print(f"failures csv: {failures_path} ({n_fail} rows)")
    con.close()
    return 1 if fail else 0


def main() -> int:
    here = Path(__file__).resolve().parent
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--base-url",
        default=os.environ.get("DIARCH_URL", "http://127.0.0.1:8083"),
    )
    p.add_argument("--user", default=None)
    p.add_argument("--password", default=None)
    p.add_argument(
        "--manifest",
        type=Path,
        default=None,
        help="Default: <staging>/epub_manifest.jsonl or ./out/epub_manifest.jsonl",
    )
    p.add_argument(
        "--staging",
        type=Path,
        default=Path(os.environ.get("DIARCH_CALIBRE_STAGING", "/var/tmp/diarch-calibre-epubs")),
    )
    p.add_argument(
        "--ledger",
        type=Path,
        default=None,
        help="Default: <staging>/import_ledger.sqlite",
    )
    p.add_argument(
        "--out",
        type=Path,
        default=None,
        help="Default: <staging>/out",
    )
    p.add_argument("--concurrency", type=int, default=2)
    p.add_argument("--poll-timeout", type=float, default=1800.0)
    p.add_argument("--poll-interval", type=float, default=2.0)
    p.add_argument(
        "--limit",
        type=int,
        default=0,
        help="Only process first N non-skipped books (0 = all)",
    )
    args = p.parse_args()

    staging = args.staging
    if args.manifest is None:
        staged_manifest = staging / "epub_manifest.jsonl"
        local_manifest = here / "out" / "epub_manifest.jsonl"
        args.manifest = staged_manifest if staged_manifest.is_file() else local_manifest
    if args.ledger is None:
        args.ledger = staging / "import_ledger.sqlite"
    if args.out is None:
        args.out = staging / "out"

    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
