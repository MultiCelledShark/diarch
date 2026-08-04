#!/usr/bin/env bash
# Stage Calibre EPUBs (hardlink/copy) then rsync the flat staging dir to keystone.
#
# Usage:
#   ./rsync_to_keystone.sh
#   ./rsync_to_keystone.sh user@keystone:/var/tmp/diarch-calibre-epubs/
#   REMOTE=keystone:/var/tmp/diarch-calibre-epubs/ ./rsync_to_keystone.sh
#
# Also rsyncs out/epub_manifest.jsonl next to the staging files as ../manifest/
# sibling... actually places manifest inside the remote dir as epub_manifest.jsonl.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STAGING="${STAGING:-$HERE/staging}"
MANIFEST="${MANIFEST:-$HERE/out/epub_manifest.jsonl}"
REMOTE="${1:-${REMOTE:-keystone:/var/tmp/diarch-calibre-epubs/}}"

if [[ ! -f "$MANIFEST" ]]; then
  echo "error: run scan_calibre.py first (missing $MANIFEST)" >&2
  exit 1
fi

python3 "$HERE/stage_epubs.py" --manifest "$MANIFEST" --staging "$STAGING"

# Ensure remote path ends with /
case "$REMOTE" in
  */) ;;
  *) REMOTE="${REMOTE}/" ;;
esac

echo "rsync → $REMOTE"
rsync -avh --progress \
  --include='*.epub' \
  --exclude='*' \
  "$STAGING/" "$REMOTE"

echo "rsync manifest → ${REMOTE}epub_manifest.jsonl"
rsync -avh --progress "$MANIFEST" "${REMOTE}epub_manifest.jsonl"

echo "done. On keystone, run import_epubs.py against this staging dir."
