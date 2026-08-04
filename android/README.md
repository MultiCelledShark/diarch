# Diarch Android client

Thin Kotlin/Compose client against `http://<host>:8083` (LAN/VPN).

## Capabilities

- **Login** with editable **server URL**, username, and password (Bearer token; URL remembered after logout)
- Browse shelves: Currently Reading, To Read, Library, Wishlist, Finished
- Move works between shelves (server 409 when a capped shelf is full)
- Import files via the system document picker → `POST /api/library/import`
- **Wishlist** add by ISBN/title + **camera barcode** scan (ML Kit)
- **Offline copies** — Save EPUB / markdown / audiobook (+ cover, chapters) on-device from work detail; reader and player prefer local files; shelves fall back to downloaded titles when the server is unreachable
- **In-app reader**
  - Paginated EPUB (epub.js in a WebView): page turns, TOC, progress CFI, RTL/manga
  - **Infinite scroll** switches to a justified markdown stream
  - Typography + infinite-scroll preference synced via `/api/settings`
- **Audiobook player** (Media3): stream or play local M4B, chapters, `mode=audio` progress; chapter changes jump the text TOC when titles match

Not included: in-app document editing, KOReader sync, cover/admin tooling.

## Stack

Kotlin, Jetpack Compose, Material 3, Navigation, Retrofit/OkHttp, DataStore, Coil, Media3, CameraX + ML Kit barcode, WebView reader assets.

## Build

Requires JDK 17+ and Android SDK (compile/target 36).

```bash
cd android
echo "sdk.dir=$ANDROID_HOME" > local.properties   # if needed
./gradlew :app:assembleDebug
```

APK: `app/build/outputs/apk/debug/app-debug.apk`

## Pointing at Keystone

On the login screen set **Server URL** to your Diarch host, e.g. `http://keystone:8083` or `http://192.168.x.x:8083`. Cleartext HTTP is allowed for LAN/VPN.
