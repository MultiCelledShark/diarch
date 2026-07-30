# Diarch

Personal ebook / audiobook library. Rust (Axum + SQLite), multi-user ACL, EPUB + Markdown storage, StoryGraph flags, reMarkable send, LocalAI covers.

## Quick start (Arch laptop)

```bash
export DIARCH_DATA_DIR=./data
export DIARCH_ADMIN_USER=admin
export DIARCH_ADMIN_PASS='change-me'
cargo run -p diarch-server
```

Open `http://127.0.0.1:8083` — default listen `0.0.0.0:8083`.

## Environment

| Variable | Default | Meaning |
|----------|---------|---------|
| `DIARCH_LISTEN` | `0.0.0.0:8083` | Bind address |
| `DIARCH_DATA_DIR` | `./data` | DB + library (Keystone: `/var/lib/diarch`) |
| `DIARCH_ADMIN_USER` / `DIARCH_ADMIN_PASS` | `admin` / `admin` | Bootstrap admin |
| `DIARCH_LOCALAI_URL` | — | LocalAI base URL for covers |
| `DIARCH_HERMES_URL` | — | Hermes agent base (optional) |
| `DIARCH_STORYGRAPH_USER` / `DIARCH_STORYGRAPH_COOKIE` | — | StoryGraph pull |
| `DIARCH_REMARKABLE_TOKEN` | — | Optional; prefer `rmapi` on PATH |
| `DIARCH_SHOW_AUDIO_GAPS` | `true` | Soft audio-gap badges |

## Deploy to Keystone (Debian 13)

Develop on Arch; **do not** copy a glibc-linked Arch binary blindly.

```bash
# musl static (recommended)
rustup target add x86_64-unknown-linux-musl
cargo build -p diarch-server --release --target x86_64-unknown-linux-musl
scp target/x86_64-unknown-linux-musl/release/diarch key@keystone:/tmp/
# on Keystone:
sudo install -m 755 /tmp/diarch /usr/local/bin/diarch
sudo apt install pandoc
sudo mkdir -p /var/lib/diarch
sudo cp deploy/debian/diarch.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now diarch
```

See [deploy/debian/README.md](deploy/debian/README.md).

## Layout

- `crates/diarch-server` — HTTP API + embedded web UI
- `crates/diarch-db` — SQLite
- `crates/diarch-import` — pandoc import / confirm
- `crates/diarch-core` — domain + config
- `taxonomy/seed.json` — 1000–9999 codes
- `web/` — UI source (copied to `web/dist` for `rust-embed`)
- `android/` — thin client notes / stub
- `docs/TODO-ebook2audiobook-watcher.md` — desktop TTS watcher (out of scope here)

## Tests

```bash
cargo test --workspace
```

Coverage includes:

- **diarch-core** — taxonomy seed, primary-code suggestion, manga inference, title matching
- **diarch-db** — auth/sessions, ACL grants, multi-code works, attention filters, jobs
- **diarch-import** — markdown ingest, zip export, TTS queue copy; pandoc confirm when installed
- **diarch-server** — HTTP API regression (auth, ACL, wishlist, flags, progress, settings, import jobs)

Pandoc-dependent cases skip cleanly if `pandoc` is missing.
