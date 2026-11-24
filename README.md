# Rustify

A native Spotify desktop client for Windows, written in Rust.

Rustify reimplements the Spotify desktop app with a native [Slint](https://slint.dev) interface. Audio is played through Spotify's official Web Playback SDK, so it works with any Spotify Premium account. A built-in plugin system lets anyone extend it, with an API modeled on Spicetify.

Rustify is an independent project. It is not affiliated with or endorsed by Spotify.

## Why Rustify

Most third-party Spotify clients, including Spotifast, play audio through [librespot](https://github.com/librespot-org/librespot). librespot needs Spotify to hand out a decryption key for every track, and Spotify refuses those requests for many accounts, especially newer ones. When that happens playback fails, and there is nothing a user can do to fix it.

Rustify takes a different approach:

- **Playback uses Spotify's official Web Playback SDK**, the same player behind open.spotify.com. Spotify supports it for every Premium account, so playback works for everyone.
- **librespot is used only for metadata**, such as your library, Liked Songs, lyrics and artist details. It never handles audio, so the audio key problem doesn't apply.
- **Rustify keeps working when the Web API is rate limited.** Library, Liked Songs, playlists and profile data fall back to Spotify's internal endpoints.
- **It's a native app.** The interface is a Rust and Slint application rather than a browser wrapper. It uses near-zero CPU when idle.
- **Plugins and themes are built in.** No patching the client, no reinstalling after updates. Install from GitHub or drag a file onto the window.

| | Rustify | librespot-based clients |
| --- | --- | --- |
| Works with every Premium account | Yes | No, depends on audio key access |
| Spotify Connect, DJ, lyrics | Yes | Varies |
| Survives Web API rate limits | Yes | Usually not |
| Native UI | Yes | Varies |
| Plugins and themes | Built in, sandboxed | No |

## Features

- Home, search, Your Library, playlists, albums, artists, podcasts, Liked Songs, What's New and Recents
- Playback controls, queue, shuffle, repeat, media keys and Windows media overlay
- Spotify Connect: see other devices, transfer playback, control remote playback
- Playlist management: create, edit, delete, add and remove tracks
- Spotify DJ, synced lyrics, credits and friend activity
- 6-band equalizer with presets, volume normalization, Very High / High streaming quality
- Miniplayer, and a full screen view with synced lyrics
- Plugins and themes: install from a file, GitHub or drag and drop, with a safety check and a Spicetify compatibility layer
- Disk caching and request pacing to stay within Spotify API rate limits

## Plugins and themes

Rustify has a native add-on system with a Spicetify-style API. Turn it on under Settings, **Plugins and themes**, and a Plugins & themes page appears next to your notifications.

Plugins are plain JavaScript running in a sandboxed engine inside the app. They can:

- add buttons to the top bar and the player bar, with their own icons
- add items to track, album, playlist and artist menus
- add full pages built from Rustify's own components, so they look like the rest of the app
- react to song changes, play and pause, and progress
- control playback, read Spotify Web API data and keep their own settings

Themes recolor the whole app and switch instantly. They're colors only and can't run code.

```js
Rustify.Player.addEventListener("songchange", data => {
  Rustify.showNotification("Now playing " + data.track.name);
});

Rustify.UI.Playbar.Button("Skip", "next", () => Rustify.Player.next());
```

Install add-ons from a `.zip`, from a GitHub link, by dragging a file or folder onto the window, or by copying a folder into the plugins folder. Rustify picks up new and changed add-ons while running, and edits to a running plugin load instantly.

Every add-on gets a safety check before you turn it on. Rustify looks for hidden or encoded code, internet access, profile reads and actions taken without you clicking, and archives containing programs are refused. Each add-on's page lists exactly what it can do, its findings, the Spotify data it asks for, and links to its author and source code.

Each plugin runs in its own engine with memory and time limits, permission checks and rate limits. A broken plugin is stopped without affecting the app.

Spicetify extensions can be ported with small changes: `Spicetify.Player`, `ContextMenu.Item`, `Topbar.Button`, `Playbar.Button`, `LocalStorage`, `showNotification` and `CosmosAsync.get` are provided.

See the [plugin guide](docs/PLUGINS.md) for the full API and the [example plugins and themes](examples/plugins).

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

User data is stored in `%LOCALAPPDATA%\Rustify`. Plugins live in `%LOCALAPPDATA%\Rustify\plugins`.

## Security

- Tokens are encrypted at rest with Windows DPAPI.
- The local API accepts connections only from localhost, requires a per-session key, validates the Host header and all input.
- Rustify only communicates with Spotify. There is no telemetry.
- Plugins run sandboxed with no file system, network or process access. Playback control, Spotify data and theming each need a permission the user approves.

## Limitations

- Streaming quality is limited to 256 kbit/s by the Web Playback SDK. Lossless is not available.
- Offline downloads, crossfade and Jam are not supported.
- Spotify developer apps in development mode have low rate limits. Rustify caches responses to minimize requests.

## License

Released under the [MIT License](LICENSE).
