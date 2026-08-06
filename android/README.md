# Diarch Android client

Thin Kotlin/Compose client against `http://<host>:8083` (LAN/VPN).

## Capabilities

- **Login** with editable **server URL**, username, and password (Bearer token; URL remembered after logout)
- Browse shelves: Currently Reading, To Read, Library, Wishlist, Finished
- Move works between shelves (server 409 when a capped shelf is full)
- Import files via the system document picker → `POST /api/library/import`
- **Wishlist** add by ISBN/title + **camera barcode** scan (ML Kit)
- **Offline copies** — Save EPUB / markdown / audiobook (+ cover, chapters) on-device from work detail; reader and player prefer local files; shelves fall back to downloaded titles when the server is unreachable
- **In-app updates** — checks Forgejo `key/Diarch` releases on launch and offers download/install; **⋮ → Check for updates** on shelves
- **In-app reader**
  - Paginated EPUB (epub.js in a WebView): page turns, TOC, progress CFI, RTL/manga
  - **Infinite scroll** switches to a justified markdown stream
  - Typography + infinite-scroll preference synced via `/api/settings`
- **Audiobook player** (Media3): stream or play local M4B, chapters, `mode=audio` progress; chapter changes jump the text TOC when titles match

Not included: in-app document editing, KOReader sync, cover/admin tooling.

## Stack

Kotlin, Jetpack Compose, Material 3, Navigation, Retrofit/OkHttp, DataStore, Coil, Media3, CameraX + ML Kit barcode, WebView reader assets.

## Build

Requires JDK 17+ and Android SDK (compile/target 36). Builds **arm64-v8a only** (Fairphone 6 / modern phones).

```bash
cd android
echo "sdk.dir=$ANDROID_HOME" > local.properties   # if needed
./gradlew :app:assembleDebug
```

Debug APK output: `app/build/outputs/apk/debug/app-debug.apk`

Checked-in installable build (arm64): [`dist/diarch-arm64-v8a-debug.apk`](dist/diarch-arm64-v8a-debug.apk)

## Pointing at Keystone

On the login screen set **Server URL** to your Diarch host, e.g. `http://keystone:8083` or `http://192.168.x.x:8083`. Cleartext HTTP is allowed for LAN/VPN.

## App updates (Forgejo releases)

On launch the app checks Forgejo for a newer Android build and offers to download/install it. You can also use **⋮ → Check for updates** on the shelves screen.

Default release source (overridable at build time):

| Setting | Default |
|---------|---------|
| Forgejo URL | `http://192.168.0.102:3000` |
| Repo | `key/Diarch` |
| API | `GET /api/v1/repos/key/Diarch/releases/latest` |

```bash
./gradlew :app:assembleDebug -PforgejoUrl=https://git.example.com
```

### Publishing a release

1. Bump `versionCode` / `versionName` in `app/build.gradle.kts`.
2. Build the APK:

```bash
cd android
./gradlew :app:assembleDebug
cp app/build/outputs/apk/debug/app-debug.apk dist/diarch-arm64-v8a.apk
```

3. Create a Forgejo release (UI or API) with:
   - **Tag:** `android-v{versionName}` (e.g. `android-v0.2.0`)
   - **Body:** include a `versionCode:` line so the client can compare reliably:

```text
versionCode: 2

- Fix offline EPUB loading
- Per-account shelves
```

   - **Asset:** attach `diarch-arm64-v8a.apk` (any `*.apk` works; names containing `arm64` are preferred)

Helper script (needs a Forgejo API token with `write:repository`):

```bash
export FORGEJO_TOKEN=…          # or pass --token
export FORGEJO_URL=http://192.168.0.102:3000
./android/scripts/publish-android-release.sh
```

The installed app must be allowed to install unknown apps (Android will prompt on first update). The repo’s latest release (or at least its APK asset) must be downloadable from the phone without auth — use a public repo, or a token-less release asset URL on your LAN.