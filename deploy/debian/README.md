# Debian 13 (Keystone) deploy notes

- OS: **Debian 13**, not Arch — build with `x86_64-unknown-linux-musl` or inside a Debian 13 container.
- Data: `/var/lib/diarch` (`diarch.db`, `library/`, `queue/needs_tts/`, `queue/incoming_audio/`).
- Port: **8083/tcp** (free on Keystone; 22/53/80/3000/8090/19999 taken).
- Access: open URL `http://keystone:8083` on VPN LAN (same pattern as Pi-hole / DokuWiki).

## Runtime dependency checklist

Everything Diarch shells out to or calls over the network. Install on **Keystone** (and keep the Arch laptop in sync for local runs).

### Required for core library (import + readers)

| Need | Debian package / install | Provides |
|------|--------------------------|----------|
| Pandoc | `pandoc` | EPUB ↔ Markdown, PDF HTML→MD |
| OCR | `ocrmypdf` | PDF OCR (`ocrmypdf --skip-text`) |
| Tesseract eng | `tesseract-ocr`, `tesseract-ocr-eng` | Engines/data used by ocrmypdf |
| Poppler | `poppler-utils` | `pdftohtml` + `pdftotext` (fallback when HTML extract is empty) |

```bash
sudo apt-get install -y pandoc ocrmypdf tesseract-ocr tesseract-ocr-eng poppler-utils
```

### Required for Audible AAX → M4B

| Need | Debian package / install | Notes |
|------|--------------------------|-------|
| ffmpeg | `ffmpeg` | Also provides `ffprobe` (chapter listing). Required whenever users upload `.aax`. |
| Activation bytes | env `DIARCH_AUDIBLE_KEY` | Never commit; set in the systemd unit or a root-owned env file. |

```bash
sudo apt-get install -y ffmpeg
```

AAX convert needs free disk under `/var/lib/diarch` (temp `.m4b` beside the work); keep headroom for large audiobooks.

### Required for Send to reMarkable

| Need | Install | Notes |
|------|---------|-------|
| **rmapi** | Binary from [ddvk/rmapi releases](https://github.com/ddvk/rmapi/releases) → `/usr/local/bin/rmapi` | **Not in apt.** Diarch only shells out to `rmapi put`; no HTTP-only upload path. |
| Cloud auth | One-time login **as user `diarch`** | Tokens in `RMAPI_CONFIG=/var/lib/diarch/.rmapi` (set in the unit). |

`DIARCH_REMARKABLE_TOKEN` is unused for uploads.

### Optional integrations (env + egress; no extra apt)

| Feature | Config | Needs |
|---------|--------|-------|
| StoryGraph sync | `DIARCH_STORYGRAPH_USER`, `DIARCH_STORYGRAPH_COOKIE` (`remember_user_token`) | Outbound HTTPS to `app.thestorygraph.com` |
| Google Books enrich | `DIARCH_GOOGLE_BOOKS_KEY` | Outbound HTTPS to `googleapis.com` (unauthenticated quota is easy to hit) |
| Open Library / LoC | none | Outbound HTTPS to `openlibrary.org`, Library of Congress |
| LocalAI / Hermes covers + transcription | `DIARCH_LOCALAI_URL` / `DIARCH_HERMES_URL` | **Phase 8** — not required yet |

Outbound HTTPS from Keystone is required for metadata enrich, StoryGraph pull, cover fetch, and rmapi cloud auth/upload. Library browse/read of already-imported files works offline.

### Not required on the server

| Tool | Why |
|------|-----|
| `uv` / AUR helpers | Laptop-only ways to get ocrmypdf/rmapi on Arch |
| Android / KOReader clients | Phase 7 |
| ebook2audiobook watcher | Separate desktop project; drops `.m4b` into `queue/incoming_audio` |

## First-time setup

```bash
sudo useradd --system --home /var/lib/diarch --shell /usr/sbin/nologin diarch || true
sudo mkdir -p /var/lib/diarch
sudo chown -R diarch:diarch /var/lib/diarch
sudo apt-get install -y \
  pandoc ocrmypdf tesseract-ocr tesseract-ocr-eng poppler-utils ffmpeg
sudo install -m 755 diarch /usr/local/bin/diarch
sudo cp diarch.service /etc/systemd/system/
# edit Environment= passwords / optional keys in the unit (see below)
sudo systemctl daemon-reload
sudo systemctl enable --now diarch
sudo systemctl status diarch
```

## rmapi (required for Send to reMarkable)

```bash
# Bump version from https://github.com/ddvk/rmapi/releases as needed
curl -fsSL -o /tmp/rmapi.tgz \
  "https://github.com/ddvk/rmapi/releases/download/v0.0.34/rmapi-linux-amd64.tar.gz"
sudo tar -xzf /tmp/rmapi.tgz -C /usr/local/bin rmapi
sudo chmod 755 /usr/local/bin/rmapi
/usr/local/bin/rmapi version
```

Authenticate **once as the `diarch` service user**:

```bash
# Paste the 8-char code from https://my.remarkable.com/device/browser/connect
sudo -u diarch -H env RMAPI_CONFIG=/var/lib/diarch/.rmapi /usr/local/bin/rmapi
sudo -u diarch -H env RMAPI_CONFIG=/var/lib/diarch/.rmapi /usr/local/bin/rmapi ls
```

Without this login, **Send to reMarkable** fails even if Admin → Integrations shows the binary as present.

## Optional env on the unit

Add to `/etc/systemd/system/diarch.service` (then `daemon-reload` + restart) as needed:

```ini
Environment=DIARCH_AUDIBLE_KEY=your-activation-bytes
Environment=DIARCH_GOOGLE_BOOKS_KEY=your-google-books-key
Environment=DIARCH_STORYGRAPH_USER=your_sg_username
Environment=DIARCH_STORYGRAPH_COOKIE=remember_user_token_value
# Phase 8:
# Environment=DIARCH_LOCALAI_URL=http://127.0.0.1:8080
```

Prefer a root-owned `EnvironmentFile=/etc/diarch.env` (`chmod 600`) instead of putting secrets in the unit file if the unit is world-readable.

## Verify after deploy

```bash
# CLI tools visible to the service PATH
sudo -u diarch -H env PATH=/usr/local/bin:/usr/bin \
  bash -lc 'for b in pandoc ocrmypdf pdftohtml pdftotext ffmpeg ffprobe rmapi; do command -v $b || echo MISSING $b; done'

# Admin UI → Integrations → Probe all
# Or: curl -sS -b "diarch_session=…" http://127.0.0.1:8083/api/integrations
```
