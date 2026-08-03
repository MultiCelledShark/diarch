# Diarch

Personal ebook / audiobook library. Rust (Axum + SQLite), multi-user ACL, EPUB + Markdown storage, M4B audio (Audible AAX convert), StoryGraph flags, reMarkable send, LocalAI covers.

## Quick start (Arch laptop)

**PDF import requires** `pandoc`, `ocrmypdf` (Tesseract), and Poppler (`pdftohtml`) on `PATH`.  
**reMarkable send requires** [`rmapi`](https://github.com/ddvk/rmapi) on `PATH` (same on Keystone — see deploy notes).

```bash
# Arch
sudo pacman -S pandoc poppler tesseract tesseract-data-eng
# ocrmypdf: AUR, or:
uv tool install ocrmypdf   # ensure ~/.local/bin is on PATH

# rmapi — required for Send to reMarkable (same binary needed on Keystone)
# AUR: yay -S rmapi
# Or release binary:
mkdir -p ~/.local/bin
curl -fsSL -o /tmp/rmapi.tgz \
  "https://github.com/ddvk/rmapi/releases/download/v0.0.34/rmapi-linux-amd64.tar.gz"
tar -xzf /tmp/rmapi.tgz -C ~/.local/bin rmapi
# One-time auth (paste code from https://my.remarkable.com/device/browser/connect):
rmapi
```

Pandoc 3.x cannot read PDF directly; Diarch OCRs with `ocrmypdf --skip-text`, converts via `pdftohtml`, then `pandoc` HTML→Markdown.
```bash
export DIARCH_DATA_DIR=./data
export DIARCH_ADMIN_USER=admin
export DIARCH_ADMIN_PASS='change-me'
export PATH="$HOME/.local/bin:$PATH"
cargo run -p diarch-server
```

Open `http://127.0.0.1:8083` — default listen `0.0.0.0:8083`.

## Environment

Optional local file: copy [`.env.example`](.env.example) → `.env` (gitignored). `dotenvy` loads it on startup; real process env still wins.

| Variable | Default | Meaning |
|----------|---------|---------|
| `DIARCH_LISTEN` | `0.0.0.0:8083` | Bind address |
| `DIARCH_DATA_DIR` | `./data` | DB + library (Keystone: `/var/lib/diarch`) |
| `DIARCH_ADMIN_USER` / `DIARCH_ADMIN_PASS` | `admin` / `admin` | Bootstrap admin |
| `DIARCH_LOCALAI_URL` | — | LocalAI base URL (covers + transcription) |
| `DIARCH_LOCALAI_IMAGE_MODEL` | `flux.2-klein-4b` when URL set | Image model id for Generate cover |
| `DIARCH_LOCALAI_IMAGE_SIZE` | `512x512` when URL set | Cover size (`WxH`); lower if GPU OOMs |
| `DIARCH_LOCALAI_TRANSCRIBE_MODEL` | `nemo-parakeet-tdt-0.6b` when URL set | ASR model for Transcribe |
| `DIARCH_HERMES_URL` | — | Optional Hermes agent for covers |
| `DIARCH_AUDIBLE_KEY` | — | Audible activation bytes for AAX → M4B |
| `DIARCH_GOOGLE_BOOKS_KEY` | — | Google Books API key (avoids unauthenticated 429; needed for many indie/recent titles OL/LoC lack) |
| `DIARCH_STORYGRAPH_USER` / `DIARCH_STORYGRAPH_COOKIE` | — | StoryGraph bootstrap (optional; Integrations UI writes `data/storygraph.conf`) |
| `DIARCH_REMARKABLE_TOKEN` | — | Unused for upload; **`rmapi` on PATH is required** for Send to reMarkable |
| `DIARCH_SHOW_AUDIO_GAPS` | `true` | Soft audio-gap badges |

## Deploy to Keystone (Debian 13)

Develop on Arch; **do not** copy a glibc-linked Arch binary blindly.

```bash
# musl static (recommended)
rustup target add x86_64-unknown-linux-musl
cargo build -p diarch-server --release --target x86_64-unknown-linux-musl
scp target/x86_64-unknown-linux-musl/release/diarch key@keystone:/tmp/
# on Keystone — full dependency checklist in deploy/debian/README.md
sudo install -m 755 /tmp/diarch /usr/local/bin/diarch
sudo apt install pandoc ocrmypdf tesseract-ocr tesseract-ocr-eng poppler-utils ffmpeg
# also: rmapi binary + auth as diarch user; optional DIARCH_* keys in the unit
sudo mkdir -p /var/lib/diarch
sudo cp deploy/debian/diarch.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now diarch
```

See **[deploy/debian/README.md](deploy/debian/README.md)** for the Keystone dependency checklist (`pandoc` / OCR / poppler / `ffmpeg`+`ffprobe` / `rmapi`, env vars, outbound HTTPS).

## Layout

- `crates/diarch-server` — HTTP API + embedded web UI
- `crates/diarch-db` — SQLite
- `crates/diarch-import` — pandoc + ocrmypdf PDF/MD import / confirm
- `crates/diarch-core` — domain + config
- `taxonomy/seed.json` — 1000–9999 codes
- `web/` — UI source (copied to `web/dist` for `rust-embed`)
- `android/` — thin client notes / stub
- `docs/TODO-ebook2audiobook-watcher.md` — desktop TTS watcher (out of scope here)
- `docs/PHASED_PLAN.md` — phased roadmap and locked product rules

## Features by phase

See [docs/PHASED_PLAN.md](docs/PHASED_PLAN.md) for status and next priorities.

```bash
cargo test --workspace
```

Coverage includes:

- **diarch-core** — taxonomy seed, primary-code suggestion, manga inference, title matching
- **diarch-db** — auth/sessions, ACL grants, multi-code works, attention filters, jobs
- **diarch-import** — markdown ingest, PDF OCR (`ocrmypdf`) + `pdftohtml` + pandoc, zip export, TTS queue copy
- **diarch-server** — HTTP API regression (auth, ACL, wishlist, flags, StoryGraph/reMarkable/integrations, progress, settings, import jobs)

Pandoc / ocrmypdf / poppler-dependent cases skip cleanly if those tools are missing.
