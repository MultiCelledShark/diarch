# TODO: ebook2audiobook desktop watcher

**Status:** deferred until Diarch text import is stable.

## Goal

Keystone sets `needs_tts` on works. `POST /api/queue/needs_tts/export` stages EPUBs as:

```
/var/lib/diarch/queue/needs_tts/{Title}--{work_id}.epub
/var/lib/diarch/queue/needs_tts/manifest.json
```

Filenames and OPF title/author come from SQLite (not bare `book.epub` / pandoc’s default title `"book"`). Library storage remains `library/<id>/book.epub`.

A **separate** watcher living in / next to the [ebook2audiobook](https://github.com/DrewThomasson/ebook2audiobook) project on the desktop should:

1. Sync or mount that queue folder (Syncthing / SSH / NFS).
2. Run ebook2audiobook headless on each EPUB.
3. Drop resulting `.m4b` into `queue/incoming_audio/{work_id}.m4b` (parse id from filename suffix or `manifest.json`).
4. Diarch (future job) attaches audio and clears `needs_tts`.

Do **not** implement the watcher inside this Diarch repo until Phase 2 imports are trusted.

## Related

- Diarch soft flag: `works.needs_tts`
- Audible conversion script lives in `~/Projects/audible2m4b` (separate integration).
