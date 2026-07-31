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

- **Canonical storage:** EPUB + Markdown(+media) + optional M4A. PDFs only in the import box (not retained).
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
| **3 Readers** | API/MVP done | epub.js + MD scroll + manga RTL in web. No polish (TOC chrome, themes) |
| **4 Metadata / covers / wishlist** | API/MVP done | Open Library + LoC search, barcode wishlist, placeholder / LocalAI hooks. Cover gen needs LocalAI/Hermes configured |
| **5 Audio** | API/MVP done | M4A upload + Range stream. No Audible/transcription wiring yet |
| **6 StoryGraph / reMarkable / fixer** | API/MVP done | Pull+flags, rmapi send, health probes. Scrapers/CLI are fragile |
| **7 Android / KOReader** | Not done | Docs/stubs only |
| **Regression tests** | Done | `cargo test --workspace` (~32 tests) |

### What you should see after login

1. Top bar: **Diarch** · Library · Wishlist · Attention · Admin · Settings · username · Logout  
2. **Library** with an **Import ebook** dropzone (EPUB primary)  
3. Book cards after import, or empty-state guidance  
4. Work detail: editable title/authors/status/taxonomy/year list; grant by username (admin)

Restart `cargo run -p diarch-server` after pulling UI changes (assets are embedded at compile time), then hard-refresh the browser.

---

## Suggested next work (priority)

1. **UX harden** — empty states (in progress), year reading list UI, grant UX without pasting UUIDs.
2. **Audio pipeline** — Audible script; desktop `needs_tts` watcher in ebook2audiobook repo.
3. **Transcription** — background job ~10% CPU.
4. **Deploy** — musl/Debian 13 → Keystone systemd.
5. **Android** — thin client.
6. **KOReader** — optional sync.

---

## Architecture (quick)

```
Clients (Web :8083, Android later)
        │
        ▼
diarch-server (Axum) ── SQLite ── /var/lib/diarch/library
        │
        ├── pandoc + ocrmypdf + pdftohtml (PDF import)
        ├── Open Library / LoC
        ├── LocalAI / Hermes (covers)
        ├── StoryGraph (pull)
        └── rmapi (reMarkable)
```

## Env vars (reference)

| Variable | Purpose |
|----------|---------|
| `DIARCH_LISTEN` | Bind address (default `0.0.0.0:8083`) |
| `DIARCH_DATA_DIR` | Data root |
| `DIARCH_ADMIN_USER` / `DIARCH_ADMIN_PASS` | Bootstrap admin |
| `DIARCH_LOCALAI_URL` / `DIARCH_HERMES_URL` | Cover generation |
| `DIARCH_STORYGRAPH_USER` / `DIARCH_STORYGRAPH_COOKIE` | SG pull |
| `DIARCH_REMARKABLE_TOKEN` | Optional; prefer `rmapi` on PATH |
| `DIARCH_SHOW_AUDIO_GAPS` | Soft audio-gap badges |

---

*Update this table when a phase moves from API/MVP → polished, or when Phase 7 starts.*
