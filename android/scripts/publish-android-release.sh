#!/usr/bin/env bash
# Build the Diarch Android APK and publish it as a Forgejo release asset.
#
# Usage:
#   export FORGEJO_TOKEN=…          # required (repo write)
#   export FORGEJO_URL=http://forgejo
#   ./android/scripts/publish-android-release.sh
#   ./android/scripts/publish-android-release.sh --token … --notes "Bug fixes"
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
ANDROID_DIR="$ROOT/android"
GRADLE_FILE="$ANDROID_DIR/app/build.gradle.kts"

FORGEJO_URL="${FORGEJO_URL:-http://192.168.0.102:3000}"
FORGEJO_URL="${FORGEJO_URL%/}"
OWNER="${FORGEJO_OWNER:-key}"
REPO="${FORGEJO_REPO:-Diarch}"
TOKEN="${FORGEJO_TOKEN:-}"
NOTES=""
SKIP_BUILD=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --token) TOKEN="$2"; shift 2 ;;
    --url) FORGEJO_URL="${2%/}"; shift 2 ;;
    --owner) OWNER="$2"; shift 2 ;;
    --repo) REPO="$2"; shift 2 ;;
    --notes) NOTES="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *) echo "Unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$TOKEN" ]]; then
  echo "FORGEJO_TOKEN (or --token) is required." >&2
  echo "Create one at $FORGEJO_URL/user/settings/applications (write:repository)." >&2
  exit 1
fi

VERSION_NAME="$(grep -E 'versionName\s*=' "$GRADLE_FILE" | head -1 | sed -E 's/.*"([^"]+)".*/\1/')"
VERSION_CODE="$(grep -E 'versionCode\s*=' "$GRADLE_FILE" | head -1 | sed -E 's/[^0-9]//g')"
TAG="android-v${VERSION_NAME}"
APK_NAME="diarch-arm64-v8a.apk"
DIST_DIR="$ANDROID_DIR/dist"
APK_OUT="$DIST_DIR/$APK_NAME"

echo "Publishing Android $VERSION_NAME (versionCode $VERSION_CODE) as $TAG"

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  (
    cd "$ANDROID_DIR"
    ./gradlew :app:assembleDebug
  )
  mkdir -p "$DIST_DIR"
  cp -f "$ANDROID_DIR/app/build/outputs/apk/debug/app-debug.apk" "$APK_OUT"
fi

if [[ ! -f "$APK_OUT" ]]; then
  echo "Missing APK at $APK_OUT" >&2
  exit 1
fi

BODY="versionCode: ${VERSION_CODE}

${NOTES:-Android client ${VERSION_NAME}.}"

API="$FORGEJO_URL/api/v1/repos/$OWNER/$REPO"
AUTH="Authorization: token $TOKEN"

# Create release (ignore failure if tag already exists — then fetch it).
CREATE_PAYLOAD="$(jq -n \
  --arg tag "$TAG" \
  --arg name "Android $VERSION_NAME" \
  --arg body "$BODY" \
  '{tag_name:$tag, name:$name, body:$body, draft:false, prerelease:false}')"

RELEASE_JSON="$(curl -sS -X POST "$API/releases" \
  -H "$AUTH" -H "Content-Type: application/json" \
  -d "$CREATE_PAYLOAD" || true)"

RELEASE_ID="$(echo "$RELEASE_JSON" | jq -r '.id // empty')"
if [[ -z "$RELEASE_ID" || "$RELEASE_ID" == "null" ]]; then
  RELEASE_JSON="$(curl -sS "$API/releases/tags/$TAG" -H "$AUTH")"
  RELEASE_ID="$(echo "$RELEASE_JSON" | jq -r '.id // empty')"
fi

if [[ -z "$RELEASE_ID" || "$RELEASE_ID" == "null" ]]; then
  echo "Could not create or find release for tag $TAG:" >&2
  echo "$RELEASE_JSON" >&2
  exit 1
fi

echo "Release id=$RELEASE_ID — uploading $APK_NAME"
UPLOAD_URL="$API/releases/$RELEASE_ID/assets?name=$(printf %s "$APK_NAME" | jq -sRr @uri)"
curl -sS -X POST "$UPLOAD_URL" \
  -H "$AUTH" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @"$APK_OUT" \
  | jq '{id, name, browser_download_url, size}'

echo "Done. Latest: $FORGEJO_URL/$OWNER/$REPO/releases/tag/$TAG"
