# Diarch Android client

Thin Kotlin/Compose client against `http://<host>:8083` (LAN/VPN).

## Capabilities (v1)

- **Login** with editable **server URL**, username, and password (Bearer token; URL remembered after logout)
- Browse shelves: Currently Reading (`reading`, max 3), To Read (`to_read`, max 9), Library (`unread`), Finished (`read`)
- Move works between shelves (server 409 when a capped shelf is full)
- Import files via the system document picker → `POST /api/library/import`
- **In-app reader**
  - Paginated EPUB (epub.js in a WebView): page turns, TOC, progress CFI, RTL/manga
  - **Infinite scroll** switches to a justified markdown stream (`GET …/content/markdown`)
  - Typography + infinite-scroll preference synced via `GET`/`PUT /api/settings`

Not included: in-app document editing, audio player, wishlist/barcode, KOReader sync, cover/admin tooling.

## Stack

Kotlin, Jetpack Compose, Material 3, Navigation, Retrofit/OkHttp, DataStore, Coil, WebView reader assets (`epub.js`, marked, DOMPurify).

## Build

Requires JDK 17+ and Android SDK (compile/target 36).

```bash
cd android
echo "sdk.dir=$ANDROID_HOME" > local.properties   # if needed
./gradlew :app:assembleDebug
```

APK: `app/build/outputs/apk/debug/app-debug.apk`

Optional env for this tree’s Gradle cache:

```bash
export GRADLE_USER_HOME="$PWD/.gradle-home"
```

## Pointing at Keystone

On the login screen set **Server URL** to your Diarch host, e.g. `http://keystone:8083` or `http://192.168.x.x:8083`. Cleartext HTTP is allowed for LAN/VPN.
