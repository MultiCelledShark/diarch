# Calibre → Diarch EPUB migration

One-shot (resume-safe) toolkit to import **EPUB-only** Calibre books into Diarch on keystone, and list everything that still needs converting.

Counts are **per Calibre book**, not per format file. Multi-format folders contribute one EPUB import when an EPUB exists.

## Flow

1. **Laptop:** scan Calibre library → manifest + conversion CSV  
2. **Laptop:** stage flat `{calibre_id}.epub` files + rsync to keystone  
3. **Keystone:** import via local Diarch API with SQLite ledger (no duplicate re-imports)  
4. Convert remaining books in Calibre / by hand, then re-scan + re-import (ledger skips done ids)

## Prerequisites

- Calibre library with `metadata.db` (default `/home/igrot/Library`)
- Diarch running on keystone (`http://127.0.0.1:8083`, systemd unit up so import jobs run)
- SSH/`rsync` access to keystone
- Admin credentials (`DIARCH_ADMIN_USER` / `DIARCH_ADMIN_PASS`)

## 1. Scan (laptop)

```bash
cd scripts/calibre-migrate
python3 scan_calibre.py
# or: python3 scan_calibre.py --library /home/igrot/Library --out ./out
```

Writes:

| File | Purpose |
|------|---------|
| `out/epub_manifest.jsonl` | Import candidates (~581 EPUBs) |
| `out/needs_conversion.csv` | Books with no EPUB (~2,085) |

`needs_conversion.csv` columns: `calibre_id`, `uuid`, `title`, `authors`, `path`, `formats`, `has_pdf`, `suggested`

- `suggested=calibre_convert` — has MOBI/AZW3/DOCX/TXT/HTMLZ/etc. Calibre can usually make an EPUB  
- `suggested=hand_review` — PDF-only, KFX, DJVU, comics, archives, etc. (historic text / game books: PDF → MD → EPUB on the laptop)

## 2. Stage + rsync (laptop)

```bash
./rsync_to_keystone.sh
# or: ./rsync_to_keystone.sh user@keystone:/var/tmp/diarch-calibre-epubs/
```

This runs `stage_epubs.py` (hardlink when possible, else copy into `./staging/{id}.epub`), then rsyncs EPUBs + `epub_manifest.jsonl` to keystone.

Default remote: `keystone:/var/tmp/diarch-calibre-epubs/`  
Do **not** rsync into `/var/lib/diarch/library` — Diarch owns that tree.

Staging-only (no rsync):

```bash
python3 stage_epubs.py
```

## 3. Import (keystone)

Copy this directory (or at least `import_epubs.py`) onto keystone if it is not already in the deployed repo checkout, then:

```bash
export DIARCH_ADMIN_USER=admin
export DIARCH_ADMIN_PASS='…'
# optional: DIARCH_URL=http://127.0.0.1:8083

python3 import_epubs.py \
  --staging /var/tmp/diarch-calibre-epubs \
  --concurrency 2
```

Smoke test one book:

```bash
python3 import_epubs.py --staging /var/tmp/diarch-calibre-epubs --limit 1
```

Ledger: `/var/tmp/diarch-calibre-epubs/import_ledger.sqlite`  
Failures: `/var/tmp/diarch-calibre-epubs/out/import_failures.csv`

Re-run anytime after interrupt; rows with `status=ok` are skipped. Rows left `uploaded` resume job polling.

## 4. After conversions

1. In Calibre, convert easy formats → EPUB (or hand-build EPUBs for special cases).  
2. On the laptop: `python3 scan_calibre.py` again.  
3. `./rsync_to_keystone.sh` (only new/changed staging files).  
4. On keystone: `python3 import_epubs.py …` — ledger skips already-imported `calibre_id`s.

## Idempotency

Dedup key is **Calibre `books.id`**, stored in the sidecar ledger (not Diarch schema). Calibre UUID is kept for forensics. Deleting a Diarch work without clearing the ledger row will leave that Calibre id marked `ok` and skipped — delete the ledger row (or set status to `pending`) if you need a re-import.

## Cleanup

After the ledger shows all desired EPUBs `ok`:

```bash
# on keystone
rm -rf /var/tmp/diarch-calibre-epubs
```

Source Calibre library is never modified by these scripts.
