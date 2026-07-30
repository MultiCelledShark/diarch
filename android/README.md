# Diarch Android client

Thin client against `http://<host>:8083` (VPN).

## Planned capabilities

- Store base URL + login (session cookie or Bearer token from `POST /api/auth/login`)
- Browse ACL-filtered works, covers, download EPUB
- Stream M4A (`Range` requests)
- Wishlist: type ISBN / title, or camera barcode → `POST /api/wishlist`
- Reader: open EPUB in system reader or WebView with epub.js; manga RTL when `is_manga` / `reading_direction=rtl`

## Suggested stack

Kotlin + Jetpack Compose, OkHttp/Retrofit, CameraX + ML Kit barcode.

## KOReader (optional)

Prefer self-hosted [koreader-sync-server](https://github.com/koreader/koreader-sync-server) beside Diarch, or add compatible `/users/*` + `/syncs/progress` endpoints later. Progress can be mirrored into Diarch `reading_progress` when desired.

This directory is a stub; implement the app in a follow-up once the API is deployed on Keystone.
