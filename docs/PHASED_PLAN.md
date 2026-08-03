# Diarch — Phased Development Plan

Personal ebook / audiobook library. **Rust** (Axum + SQLite), multi-user ACL, deploy Arch laptop → **Debian 13 Keystone**.

| Runtime | Value |
|---------|--------|
| Listen | `0.0.0.0:8083` |
| Data dir (Keystone) | `/var/lib/diarch` |
| Local override | `DIARCH_DATA_DIR=./data` |
| Remote | `ssh://git@192.168.0.102/key/Diarch.git` |
| Local path | `~/Projects/diarch` |

Default admin: `DIARCH_ADMIN_USER` / `DIARCH_ADMIN_PASS` (defaults `admin` / `admin`).

**How to read status:** *API/MVP* means endpoints and a minimal web shell exist. It does **not** mean polished UX or production-hardened integrations.

---

## Locked product rules

- **Canonical storage:** EPUB + Markdown(+media) + optional M4B as `book.epub` / `book.md` / `book.m4b` under `library/<work-id>/`. PDFs only in the import box (not retained). **Exports** (download, reMarkable, TTS queue) take the human filename from the SQLite title at export time — do not rename on-disk assets.
- **PDF path:** PDF → OCR (`ocrmypdf`) → Markdown via pdftohtml+pandoc (`needs_review`) → confirm → EPUB via pandoc.
- **EPUB path:** store EPUB; derive Markdown with pandoc.
- **Taxonomy:** multi-code on every work; **primary** = most specific subgenre (e.g. Epic Fantasy `8201`).
- **StoryGraph:** pull + flags only (no write-back required); manual clear after you update SG.
- **Readers:** default EPUB; optional infinite-scroll Markdown; manga RTL when `89xx` / `is_manga`.
- **Access:** bind open port; clients use full URL (VPN is network, not app logic).
- **Modest flags:** Attention list + toggles (e.g. show audio gaps); no noisy banners.
- **ebook2audiobook watcher:** separate project TODO — see [TODO-ebook2audiobook-watcher.md](TODO-ebook2audiobook-watcher.md).

### Taxonomy ranges

| Range | Block |
|-------|--------|
| 1000–5999 | Academic (Outline of academic disciplines) |
| 6000–6999 | Practical / Hobby / Self-help |
| 7000–7999 | Religion / Spirituality |
| 8000–8999 | Fiction (Fantasy ~8200–8399, SF ~8400–8599, Comics/Manga/YA/NA) |
| 9000–9999 | Biography / Memoir |

---

## Phase completion (honest)

| Phase | Claim | Reality |
|-------|--------|---------|
| **0 Bootstrap** | Done | Repo, workspace, seed, deploy notes |
| **1 Auth + works + ACL + web shell + EPUB import** | **Complete** | Login, Library with import dropzone, metadata editor (status / taxonomy / year list), grants by username, wishlist, admin users. EPUB import extracts title/author and lands the book in the library. |
| **2 Import polish** | **Complete** | PDF quarantine + side-by-side MD review; `ocrmypdf --skip-text` → `pdftohtml` → pandoc HTML→MD; PUT markdown; ISBN / title+author metadata enrich; confirm → EPUB |
| **3 Readers** | **Complete** | epub.js + MD scroll + manga RTL; TOC panel; full typography suite (palette/font/size/line-height/margins/justify); review MD toolbar + live preview |
| **4 Metadata / covers / wishlist** | **Complete** | OL + Google Books + LoC; ISBN authors fixed; remote cover fetch; wishlist enrich + barcode; extract/placeholder/upload covers. LocalAI/Hermes deferred → Phase 8 |
| **5 Audio** | **Complete** | Canonical `book.m4b` upload + Range stream; Audible AAX→M4B server job (`DIARCH_AUDIBLE_KEY` + ffmpeg, same flags as audible2m4b). Transcription stubbed → Phase 8 |
| **6 StoryGraph / reMarkable / fixer** | **Complete** | SG pull + flags; Integrations for all users (health, Probe, SG sync); reMarkable connect URL + in-app 8-char code auth; rmapi send to `Diarch/`; Keystone dep docs. Scrapers/CLI remain upstream-fragile |
| **7 Android / KOReader** | Skipped for now | Docs/stubs only — deferred |
| **8 Deferred AI + polish** | **In progress** | LocalAI covers (staged approve), transcription, health probe |
| **Regression tests** | Done | `cargo test --workspace` (core/db/import/server; Phase 6 coverage included) |

### What you should see after login

1. Top bar: **Diarch** · Library · Wishlist · Attention · **Integrations** · Admin · Settings · username · Logout  
2. **Library** with **Import** (EPUB/PDF/MD and `.m4b`/`.aax` audiobooks)  
3. Book cards after import, or empty-state guidance  
4. Work detail: editable title/authors/status/taxonomy/year list; grant by username (admin); **Upload audiobook**; **Send to reMarkable** when EPUB exists; clear StoryGraph flags after you update SG  
5. **Integrations** (all users): reMarkable connect URL + code entry, health table, Probe all, Sync StoryGraph

Restart `cargo run -p diarch-server` after pulling UI changes (assets are embedded at compile time), then hard-refresh the browser.

---

## Suggested next work (priority)

1. **Phase 8 finish** — exercise Generate cover + Approve against TrueNAS LocalAI; long-audiobook transcription timing; remaining polish below.
2. **UX harden** — empty states (in progress), year reading list UI, grant UX without pasting UUIDs.
3. **ebook2audiobook watcher** — desktop TTS → `queue/incoming_audio` as `.m4b`.
4. **Deploy** — musl/Debian 13 → Keystone systemd (deps: pandoc, ocrmypdf, tesseract, poppler-utils, ffmpeg, rmapi — see `deploy/debian/README.md`).
5. **Android / KOReader** — deferred (Phase 7).

---

## Architecture (quick)

```
Clients (Web :8083, Android later)
        │
        ▼
diarch-server (Axum) ── SQLite ── /var/lib/diarch/library
        │
        ├── pandoc + ocrmypdf + pdftohtml (PDF import)
        ├── Open Library / Google Books / LoC
        ├── ffmpeg (Audible AAX → M4B)
        ├── LocalAI / Hermes (covers / transcription — Phase 8)
        ├── StoryGraph (pull)
        └── rmapi (reMarkable)
```

## Env vars (reference)

| Variable | Purpose |
|----------|---------|
| `DIARCH_LISTEN` | Bind address (default `0.0.0.0:8083`) |
| `DIARCH_DATA_DIR` | Data root |
| `DIARCH_ADMIN_USER` / `DIARCH_ADMIN_PASS` | Bootstrap admin |
| `DIARCH_LOCALAI_URL` / `DIARCH_HERMES_URL` | LocalAI base (+ optional Hermes cover shim) |
| `DIARCH_LOCALAI_IMAGE_MODEL` | Image model (default `flux.2-klein-4b` when URL set) |
| `DIARCH_LOCALAI_TRANSCRIBE_MODEL` | ASR model (default `nemo-parakeet-tdt-0.6b` when URL set) |
| `DIARCH_AUDIBLE_KEY` | Audible activation bytes for AAX → M4B (never commit) |
| `DIARCH_GOOGLE_BOOKS_KEY` | Google Books API key (optional; avoids unauthenticated 429) |
| `DIARCH_STORYGRAPH_USER` / `DIARCH_STORYGRAPH_COOKIE` | SG pull |
| `DIARCH_REMARKABLE_TOKEN` | Unused for upload; **`rmapi` required** on laptop and Keystone |
| `DIARCH_SHOW_AUDIO_GAPS` | Soft audio-gap badges |

---

## Phase 8 — AI integration + polish

LocalAI on the LAN (e.g. TrueNAS) drives covers and transcription. Phase 7 (Android / KOReader) is skipped for now.

### LocalAI / Hermes covers

- Wire `DIARCH_LOCALAI_URL` (+ optional `DIARCH_LOCALAI_IMAGE_MODEL`, `DIARCH_HERMES_URL`) for image generation (`POST /api/works/{id}/cover/generate`, `GET …/cover/prompt`).
- Web UI: **Generate cover** on work detail; show prompt; show candidate.
- **Staged approve:** write `cover.candidate.jpg`, preview beside current cover, then **Approve → `cover.jpg`** / Discard. Do not overwrite `cover.jpg` until approve.
- Clear `needs_cover` only on approve (or explicit dismiss).
- Health probe row for LocalAI in Integrations (`/v1/models`).

### Transcription

- **Transcribe** queues a `transcribe` job when `book.m4b` exists and LocalAI is configured.
- Worker chunks audio (~10 min) via ffmpeg and calls LocalAI `/v1/audio/transcriptions` (`DIARCH_LOCALAI_TRANSCRIBE_MODEL`).
- Result: `transcript.txt` (+ download link on work detail).

### Other optionals skipped earlier

| Origin | Item |
|--------|------|
| Phase 3 | Server-persisted reader typography (today: `localStorage` only) |
| Phase 3 | Richer MD review editor (CodeMirror/Monaco) if toolbar+preview is not enough |
| Phase 4 | Cover “reset to placeholder” / regenerate SVG from current title+authors |
| Phase 4 | Attention one-click clear for soft flags without opening detail |
| Phase 4 | **Fix metadata search / enrich** — new/indie ISBNs often missing from OL; Google needs `DIARCH_GOOGLE_BOOKS_KEY`; broaden providers / UX so Enrich is reliable beyond “not in catalog” |
| Product | ebook2audiobook desktop watcher — [TODO-ebook2audiobook-watcher.md](TODO-ebook2audiobook-watcher.md) |
| Phase 7 | Android thin client / KOReader sync — deferred |

---

*Update this table when a phase moves from API/MVP → polished, or when Phase 7 / 8 starts.*
