# Debian 13 (Keystone) deploy notes

- OS: **Debian 13 (trixie)**, not Arch — build with `x86_64-unknown-linux-musl` or inside a Debian 13 container / chroot. Do **not** copy a glibc-linked Arch laptop binary.
- Binary: crate `diarch-server` installs as **`diarch`** (`[[bin]] name = "diarch"`).
- Data: `/var/lib/diarch` (see layout below).
- Port: **8083/tcp** (free on Keystone; 22/53/80/3000/8090/19999 taken).
- Access: open URL `http://keystone:8083` on VPN LAN (same pattern as Pi-hole / DokuWiki).
- TLS: outbound HTTPS uses **rustls** (no system OpenSSL / `libssl` needed for Diarch itself).

## Data directory layout

Owned by `diarch:diarch`:

| Path | Purpose |
|------|---------|
| `diarch.db` | SQLite |
| `library/<work-id>/` | Canonical `book.epub` / `book.md` / `book.m4b`, covers |
| `imports/` | Quarantine uploads + temp MD zips |
| `queue/needs_tts/` | EPUB copies for ebook2audiobook watcher (export API) |
| `queue/incoming_audio/` | Drop zone for finished `.m4b` from desktop watcher |
| `.rmapi` | reMarkable tokens (`RMAPI_CONFIG`) |
| `storygraph.conf` | Optional SG creds from **Integrations → StoryGraph** (env is bootstrap/fallback) |

## Runtime dependency checklist

Everything Diarch shells out to or calls over the network. Install on **Keystone** (and keep the Arch laptop in sync for local runs).

### Required for core library (import + readers)

| Need | Debian package | Binary on PATH | Used for |
|------|----------------|----------------|----------|
| Pandoc | `pandoc` | `pandoc` | EPUB ↔ Markdown; HTML→MD after PDF extract |
| OCR | `ocrmypdf` | `ocrmypdf` | PDF OCR (`ocrmypdf --skip-text`) |
| Tesseract eng | `tesseract-ocr`, `tesseract-ocr-eng` | (via ocrmypdf) | Engines/data for OCR |
| Poppler | `poppler-utils` | `pdftohtml`, `pdftotext` | PDF→HTML; text fallback when HTML is empty |

```bash
sudo apt-get install -y pandoc ocrmypdf tesseract-ocr tesseract-ocr-eng poppler-utils
```

Pandoc 3.x cannot ingest PDF directly. Flow: OCR → `pdftohtml` → `pandoc` HTML→Markdown → review → confirm → EPUB via `pandoc`.

### Required for audio convert + transcription chunking

| Need | Debian package | Binary on PATH | Used for |
|------|----------------|----------------|----------|
| ffmpeg | `ffmpeg` | `ffmpeg`, `ffprobe` | AAX→M4B; M4B chapter list; LocalAI ASR chunking (~10 min slices) |
| Activation bytes | env `DIARCH_AUDIBLE_KEY` | — | AAX decrypt (never commit; unit or `EnvironmentFile`) |

```bash
sudo apt-get install -y ffmpeg
```

AAX convert and transcription need free disk under `/var/lib/diarch` (temp media beside the work). Keep headroom for large audiobooks.

Without `DIARCH_AUDIBLE_KEY`, AAX upload is rejected; plain `.m4b` attach still works. Without LocalAI URL, Transcribe is disabled but `ffmpeg`/`ffprobe` remain useful for chapters / future ASR.

### Required for Send to reMarkable

| Need | Install | Notes |
|------|---------|-------|
| **rmapi** | Binary from [ddvk/rmapi releases](https://github.com/ddvk/rmapi/releases) → `/usr/local/bin/rmapi` | **Not in apt.** Diarch shells out to `rmapi put` only (no HTTP-only upload path). |
| Cloud auth | One-time login **as user `diarch`** | Tokens in `RMAPI_CONFIG=/var/lib/diarch/.rmapi` (set in the unit). |

`DIARCH_REMARKABLE_TOKEN` is unused for uploads.

### Optional integrations (env / UI + egress; no extra apt)

| Feature | Config | Needs |
|---------|--------|-------|
| StoryGraph sync + enrich | Integrations UI → paste full Cookie header (`remember_user_token` + usually `cf_clearance`) + optional matching User-Agent into `storygraph.conf`; or `DIARCH_STORYGRAPH_USER` / `DIARCH_STORYGRAPH_COOKIE` | Outbound HTTPS to `app.thestorygraph.com`. Cloudflare clearance is IP/UA-bound and expires ~30–60m — re-paste when Sync fails. |
| Google Books enrich | `DIARCH_GOOGLE_BOOKS_KEY` | Outbound HTTPS to `googleapis.com` (unauthenticated quota is easy to hit; set a key for indie/recent titles OL/LoC lack) |
| Open Library / LoC | none | Outbound HTTPS to `openlibrary.org`, Library of Congress |
| LocalAI covers + transcription | `DIARCH_LOCALAI_URL`, optional `DIARCH_LOCALAI_IMAGE_MODEL` / `DIARCH_LOCALAI_IMAGE_SIZE` / `DIARCH_LOCALAI_TRANSCRIBE_MODEL` / `DIARCH_HERMES_URL` | Reachable from Keystone (e.g. TrueNAS LocalAI); still needs local `ffmpeg`/`ffprobe` for ASR chunking |

Outbound HTTPS from Keystone is required for metadata enrich, StoryGraph, remote cover fetch, and rmapi cloud auth/upload. Browsing already-imported files works offline.

### Not required on the server

| Tool | Why |
|------|-----|
| `ghostscript` / `gs` | Only used in some **dev tests** to synthesize PDF fixtures |
| `uv` / AUR helpers | Laptop-only ways to get ocrmypdf/rmapi on Arch |
| Android / KOReader clients | Phase 7 deferred |
| ebook2audiobook watcher | Separate desktop project; drops `.m4b` into `queue/incoming_audio` |
| System OpenSSL for Diarch | Binary uses rustls |

## Build (on Arch laptop or CI)

```bash
rustup target add x86_64-unknown-linux-musl
# may need: sudo pacman -S musl  (and a musl linker such as x86_64-linux-musl-gcc / mold setup)
cargo build -p diarch-server --release --target x86_64-unknown-linux-musl
scp target/x86_64-unknown-linux-musl/release/diarch key@keystone:/tmp/diarch
```

Alternatively build inside a Debian 13 container so the binary links against that glibc — still install the apt deps on Keystone either way.

## First-time setup

```bash
sudo useradd --system --home /var/lib/diarch --shell /usr/sbin/nologin diarch || true
sudo mkdir -p /var/lib/diarch/{library,imports,queue/needs_tts,queue/incoming_audio}
sudo chown -R diarch:diarch /var/lib/diarch
sudo apt-get update
sudo apt-get install -y \
  pandoc ocrmypdf tesseract-ocr tesseract-ocr-eng poppler-utils ffmpeg
sudo install -m 755 /tmp/diarch /usr/local/bin/diarch
# rmapi — see next section
sudo cp deploy/debian/diarch.service /etc/systemd/system/diarch.service
# Prefer secrets in a root-owned env file:
#   sudo install -m 600 /dev/null /etc/diarch.env
#   # edit /etc/diarch.env, then uncomment EnvironmentFile= in the unit
sudo systemctl daemon-reload
sudo systemctl enable --now diarch
sudo systemctl status diarch
```

## rmapi (required for Send to reMarkable)

Latest release at time of writing: **v0.0.34** (bump from [releases](https://github.com/ddvk/rmapi/releases) as needed).

```bash
curl -fsSL -o /tmp/rmapi.tgz \
  "https://github.com/ddvk/rmapi/releases/download/v0.0.34/rmapi-linux-amd64.tar.gz"
sudo tar -xzf /tmp/rmapi.tgz -C /usr/local/bin rmapi
sudo chmod 755 /usr/local/bin/rmapi
/usr/local/bin/rmapi version
```

Prefer authenticating from the web UI: **Integrations → reMarkable** (open the connect URL, paste the 8-character code). Diarch writes the same token file `rmapi` uses (`RMAPI_CONFIG`).

CLI alternative as the `diarch` service user:

```bash
# Paste the 8-char code from https://my.remarkable.com/device/browser/connect
sudo -u diarch -H env RMAPI_CONFIG=/var/lib/diarch/.rmapi /usr/local/bin/rmapi
sudo -u diarch -H env RMAPI_CONFIG=/var/lib/diarch/.rmapi /usr/local/bin/rmapi ls
```

Without this login, **Send to reMarkable** fails even if Integrations shows `rmapi` installed.

## Optional env on the unit

Add to `/etc/systemd/system/diarch.service` or prefer `EnvironmentFile=/etc/diarch.env` (`chmod 600`), then `daemon-reload` + restart:

```ini
# EnvironmentFile=/etc/diarch.env

Environment=DIARCH_AUDIBLE_KEY=your-activation-bytes
Environment=DIARCH_GOOGLE_BOOKS_KEY=your-google-books-key
Environment=DIARCH_STORYGRAPH_USER=your_sg_username
Environment=DIARCH_STORYGRAPH_COOKIE=remember_user_token_value
# LocalAI (TrueNAS example):
Environment=DIARCH_LOCALAI_URL=http://192.168.0.104:30286
# Environment=DIARCH_LOCALAI_IMAGE_MODEL=flux.2-klein-4b
# Environment=DIARCH_LOCALAI_IMAGE_SIZE=512x512
# Environment=DIARCH_LOCALAI_TRANSCRIBE_MODEL=nemo-parakeet-tdt-0.6b
# Environment=DIARCH_HERMES_URL=http://127.0.0.1:9xxx
# Environment=DIARCH_SHOW_AUDIO_GAPS=true
```

Defaults when `DIARCH_LOCALAI_URL` is set but model vars are omitted: image `flux.2-klein-4b`, size `512x512`, ASR `nemo-parakeet-tdt-0.6b`.

StoryGraph: prefer **Integrations → StoryGraph** (writes `/var/lib/diarch/storygraph.conf`). Env vars still work as bootstrap when no file exists.

## Verify after deploy

```bash
# CLI tools visible to the service PATH
sudo -u diarch -H env PATH=/usr/local/bin:/usr/bin \
  bash -lc 'for b in pandoc ocrmypdf pdftohtml pdftotext ffmpeg ffprobe rmapi; do
    command -v "$b" >/dev/null && echo "ok  $b" || echo "MISSING $b"
  done'

# Health: Admin/Integrations → Probe all
# Expect rows for: pandoc, ocrmypdf, pdftohtml, ffmpeg, openlibrary/loc, googlebooks,
# storygraph, remarkable, localai (if URL set), audible
# Or: curl -sS -b "diarch_session=…" http://127.0.0.1:8083/api/integrations

systemctl is-active diarch
curl -fsS -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8083/api/health
```
