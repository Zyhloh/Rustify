# Rustify

A native Spotify desktop client for Windows, written in Rust.

Rustify reimplements the Spotify desktop app with a native [Slint](https://slint.dev) interface. Audio is played through Spotify's official Web Playback SDK, so it works with any Spotify Premium account.

Rustify is an independent project. It is not affiliated with or endorsed by Spotify.

## Features

- Home, search, Your Library, playlists, albums, artists, podcasts, Liked Songs, What's New and Recents
- Playback controls, queue, shuffle, repeat, media keys and Windows media overlay
- Spotify Connect: see other devices, transfer playback, control remote playback
- Playlist management: create, edit, delete, add and remove tracks
- Spotify DJ, synced lyrics, credits and friend activity
- 6-band equalizer with presets, volume normalization, Very High / High streaming quality
- Miniplayer and full screen view
- Disk caching and request pacing to stay within Spotify API rate limits

## Requirements

- Windows 10 or 11, 64-bit
- [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) (included with Windows 11)
- Spotify Premium
- A Spotify developer app (free)

## Installation

Download `RustifySetup.exe` from the [latest release](https://github.com/Zyhloh/Rustify/releases/latest) and run it. Rustify installs per user and does not require administrator rights.

### First launch

Rustify uses your own Spotify developer app to sign in.

1. Create an app in the [Spotify Developer Dashboard](https://developer.spotify.com/dashboard).
2. Add the redirect URI `http://127.0.0.1:8898/login`.
3. Enable **Web API** and **Web Playback SDK**.
4. Paste the app's Client ID into Rustify and sign in.

No client secret is required. Rustify authenticates with PKCE.

## Building from source

Requires a stable [Rust](https://rustup.rs) toolchain. [Inno Setup 6](https://jrsoftware.org/isinfo.php) is optional and only needed to build the installer.

```bat
git clone https://github.com/Zyhloh/Rustify
cd Rustify
build.bat
```

Build output is written to `dist\`.

For development, `run.bat` starts the API and client with live reload using [bacon](https://github.com/Canop/bacon).

## Architecture

| Crate | Description |
| --- | --- |
| `client` | Slint desktop UI. Launches and communicates with the local API. |
| `api` | Local axum service bound to `127.0.0.1`. Handles authentication, the Spotify Web API, library data through [librespot](https://github.com/librespot-org/librespot), and playback through a hidden WebView2 running the Web Playback SDK. |

User data is stored in `%LOCALAPPDATA%\Rustify`.

## Security

- Tokens are encrypted at rest with Windows DPAPI.
- The local API accepts connections only from localhost, requires a per-session key, validates the Host header and all input.
- Rustify only communicates with Spotify. There is no telemetry.

## Limitations

- Streaming quality is limited to 256 kbit/s by the Web Playback SDK. Lossless is not available.
- Offline downloads, crossfade and Jam are not supported.
- Spotify developer apps in development mode have low rate limits. Rustify caches responses to minimize requests.

## License

Released under the [MIT License](LICENSE).
