# Rustify plugins

Rustify runs plugins and themes natively. A plugin is a folder with a `manifest.json` and a JavaScript file. It can add buttons to the top bar and player bar, add items to track menus, render its own pages, react to playback, read Spotify Web API data, store settings and recolor the app. A theme is a folder with only a `manifest.json` that lists colors.

Add-ons are off by default. Users turn them on under **Settings**, **Plugins and themes**, which adds the **Plugins & themes** page (the puzzle button in the top bar).

Plugins run in an embedded JavaScript engine (QuickJS) inside Rustify, not in a browser. There is no DOM. UI is described as data and Rustify draws it with its own components, so plugin UI matches the rest of the app.

If you are porting a Spicetify extension, read [Porting from Spicetify](#porting-from-spicetify).

- [Quick start](#quick-start)
- [Manifest](#manifest)
- [Permissions](#permissions)
- [API reference](#api-reference)
- [Pages and blocks](#pages-and-blocks)
- [Themes](#themes)
- [Installing and sharing](#installing-and-sharing)
- [Safety check](#safety-check)
- [Porting from Spicetify](#porting-from-spicetify)
- [Limits and sandbox](#limits-and-sandbox)
- [Debugging](#debugging)

## Quick start

1. In **Settings**, turn on **Enable plugins and themes**, open **Plugins & themes** and click **Open folder**. The folder is `%LOCALAPPDATA%\Rustify\plugins`.
2. Create a folder named after your plugin id, for example `hello-world`.
3. Add `manifest.json`:

   ```json
   {
     "id": "hello-world",
     "name": "Hello World",
     "version": "1.0.0",
     "author": { "name": "you", "url": "https://github.com/you" },
     "description": "Says hello when a song starts.",
     "repository": "https://github.com/you/hello-world",
     "permissions": ["player"]
   }
   ```

4. Add `index.js`:

   ```js
   Rustify.Player.addEventListener("songchange", data => {
     if (data.track) Rustify.showNotification("Now playing " + data.track.name);
   });

   Rustify.UI.Playbar.Button("Skip", "next", () => Rustify.Player.next());
   ```

5. Rustify notices the new folder within a couple of seconds and lists it on the Plugins & themes page. Turn it on and approve its permissions.

Saving a file of a running plugin reloads it automatically. If a plugin update asks for new permissions or gets new safety warnings, it is turned off until the user approves it again.

Complete examples live in [`examples/plugins`](../examples/plugins): `now-playing-tools` (icon, page, buttons, menu item, storage, Web API), and the `midnight-theme` and `ember-theme` themes.

## Manifest

| Field | Required | Description |
| --- | --- | --- |
| `id` | yes | 3 to 40 characters, `a-z`, `0-9` and `-`. The folder must have the same name. |
| `type` | no | `plugin` (default) or `theme`. |
| `name` | yes | Display name, up to 40 characters. |
| `version` | no | Shown next to the name. |
| `author` | no | A name, or `{ "name": "...", "url": "https://..." }` to link the author's page. |
| `description` | no | Up to 300 characters. |
| `repository` | no | `https://` link to the source code. Shown as **Source code** on the add-on's page. |
| `homepage` | no | `https://` link to a website. |
| `icon` | no | A `.png` or `.jpg` in the folder, up to 256 KB. Square, 256 by 256 works best. Without one, Rustify draws a letter tile. |
| `main` | no | Script file name inside the folder. Defaults to `index.js`. Themes can't have one. |
| `permissions` | no | Any of `player`, `spotify`, `theme`. |
| `theme` | no | Colors. Required for themes. A plugin needs the `theme` permission to use it. See [Themes](#themes). |

Unknown fields are rejected so typos are caught early. Links must be `https://`. The manifest may be up to 64 KB and the script up to 512 KB.

## Permissions

| Permission | Allows |
| --- | --- |
| `player` | Playback control: play, pause, skip, seek, volume, shuffle, repeat. |
| `spotify` | Read-only requests to the Spotify Web API (`GET /v1/...`) with the user's account. |
| `theme` | Changing Rustify's colors. |

Everything else needs no permission: reading the current playback state, buttons, menu items, pages, storage, notifications, timers and navigation.

When a user turns a plugin on, Rustify shows exactly what it can do and asks for confirmation.

## API reference

Everything is on the global `Rustify` object, which is frozen.

### Player

```js
Rustify.Player.data          // current state, see below
Rustify.Player.play()
Rustify.Player.pause()
Rustify.Player.togglePlay()
Rustify.Player.next()
Rustify.Player.back()
Rustify.Player.seek(ms)
Rustify.Player.setVolume(level)      // 0 to 1
Rustify.Player.toggleShuffle()
Rustify.Player.toggleRepeat()
Rustify.Player.toggleLike()           // save or remove the current song from Liked Songs
Rustify.Player.getVolume()
Rustify.Player.isPlaying()
Rustify.Player.getProgress()         // ms
Rustify.Player.getDuration()         // ms
Rustify.Player.addEventListener(type, callback)    // returns an unsubscribe function
Rustify.Player.removeEventListener(type, callback)
```

Reading state is always allowed. Control methods need the `player` permission.

`Rustify.Player.data`:

```js
{
  track: {
    id, uri, name, album, duration, image,
    artists: [{ id, name }]
  } | null,
  isPlaying, position, duration, volume, shuffle,
  repeat,     // "off", "context" or "track"
  context     // URI of the playing playlist or album, or null
}
```

Events. Each callback receives the same object as `Player.data`.

| Event | When |
| --- | --- |
| `songchange` | The track changes. |
| `onplaypause` | Playback starts or pauses. |
| `onprogress` | Every state update while playing, about once per second. |

`Rustify.on("pagechange", ({ page }) => ...)` fires when the user navigates.

### UI

```js
const button = Rustify.UI.Topbar.Button(label, icon, onClick, active)
const button = Rustify.UI.Playbar.Button(label, icon, onClick, active)
```

Top bar buttons sit next to the back and forward arrows. Player bar buttons sit at the right of the player bar. The label is the tooltip. `onClick` receives the button.

A button has `label`, `icon` and `active` properties that update the UI when set, plus `update({ label, icon, active })` and `remove()`. An active button is drawn in the accent color.

```js
const item = Rustify.UI.ContextMenu.Item(label, types, onClick)
item.remove()
```

Adds an entry to the "..." and right-click menus. `types` is a list of `track`, `album`, `playlist`, `artist`, `show` and `episode`, or empty for all. `onClick` receives `{ uri, type, title }`.

```js
const page = Rustify.UI.Page(id, title, icon)
```

Adds a page, listed in the app menu (top left "..."). See [Pages and blocks](#pages-and-blocks).

```js
const section = Rustify.UI.NowPlaying.Section(id, title)
section.render(blocks)
section.onAction((id, value) => { ... })
section.remove()
```

Adds a card to the now-playing panel on the right, below the song credits. It uses the same blocks as pages, up to 100. A plugin can add 3 sections.

```js
const card = Rustify.UI.Home.Section(id, title)
```

Same as above, shown on the Home page below the filter chips.

```js
const button = Rustify.UI.TrackRow.Button(label, icon, (button, track) => { ... })
```

Adds an icon button to every song row in playlists, albums, search, Liked Songs and history. It appears when the row is hovered, next to the duration. `track` is `{ uri, title, type }`. A plugin can add 2, and rows show at most 4 plugin buttons in total.

```js
Rustify.UI.hide("friends-button")
Rustify.UI.show("friends-button")
Rustify.UI.elements   // the names below
```

Hides one of Rustify's own buttons: `friends-button`, `news-button`, `now-playing-button`, `lyrics-button`, `queue-button`, `devices-button`, `mini-player-button`, `fullscreen-button`. Everything a plugin hid comes back when the plugin is turned off or stops.

#### Icons

Icons are built-in names or SVG path data in a 24 by 24 box.

Built-in: `play`, `pause`, `next`, `heart`, `star`, `list`, `music`, `settings`, `search`, `info`, `bolt`, `lyrics`, `download`, `plus`. The list is also available as `Rustify.UI.icons`.

You can pass a path such as `"M12 3 L21 21 H3 Z"`, or a full `<svg>` string, from which the first `d="..."` is used. Paths may only contain path commands and numbers.

### Spotify Web API

```js
const data = await Rustify.Spotify.get("/v1/me/top/tracks?limit=10");
```

Needs the `spotify` permission. Only `GET` requests to `/v1/...` are allowed. Requests go through Rustify's API layer, so they share its rate limit handling and token refresh. A plugin can make 30 requests per minute.

### Storage

```js
Rustify.Storage.get(key)           // string or null
Rustify.Storage.set(key, value)    // stored as a string
Rustify.Storage.remove(key)
Rustify.Storage.keys()
Rustify.Storage.clear()
```

Storage is private to each plugin and kept in `%LOCALAPPDATA%\Rustify\plugin-data\<id>.json`. The limit is 1 MB per plugin. Store objects with `JSON.stringify`.

### Other

```js
Rustify.showNotification(text)     // toast, at most one every 2 seconds
Rustify.navigate(uri)              // open spotify:album:..., spotify:artist:..., spotify:playlist:..., spotify:show:...
Rustify.Theme.set(colors)          // needs "theme"
Rustify.Theme.reset()
Rustify.version                    // Rustify version
Rustify.plugin                     // this plugin's id
```

`setTimeout`, `setInterval`, `clearTimeout`, `clearInterval`, `Promise`, `async`/`await` and `console` (`log`, `info`, `warn`, `error`) work as usual. Intervals are at least 100 ms.

## Pages and blocks

A page is a list of blocks. Call `render` with the full list whenever something changes. Rustify updates only what is needed.

```js
const page = Rustify.UI.Page("stats", "Listening stats", "bolt");

page.render([
  { type: "heading", text: "Settings" },
  { type: "toggle", id: "notify", text: "Notify on song change", sub: "Shows a toast", checked: true },
  { type: "slider", id: "limit", text: "Tracks", value: 10, min: 1, max: 50 },
  { type: "input", id: "name", text: "Name", placeholder: "Type here", value: "" },
  { type: "button", id: "save", text: "Save", primary: true },
]);

page.onAction((id, value) => {
  // id is the block id, value is the new toggle, slider or input value
});

page.open();
page.remove();
```

| Type | Fields | Action |
| --- | --- | --- |
| `heading` | `text` | |
| `text` | `text` | |
| `divider` | | |
| `button` | `id`, `text`, `primary` | `(id)` on click |
| `toggle` | `id`, `text`, `sub`, `checked` | `(id, true or false)` |
| `slider` | `id`, `text`, `value`, `min`, `max` | `(id, number)` when released |
| `input` | `id`, `text`, `placeholder`, `value` | `(id, string)` on Enter |
| `row` | `id`, `text`, `sub`, `image` | `(id)` on click |
| `image` | `image` | |

Images must be Spotify artwork URLs (`i.scdn.co` and the other Spotify image hosts). Other hosts are ignored. A page holds up to 300 blocks.

## Themes

A theme is a manifest with `"type": "theme"` and colors. It has no script and can't run code, so Rustify applies it without a permission prompt. Only one theme is active at a time, and choosing another one or **Rustify (default)** switches instantly.

```json
{
  "id": "ocean-theme",
  "type": "theme",
  "name": "Ocean",
  "version": "1.0.0",
  "author": { "name": "you", "url": "https://github.com/you" },
  "repository": "https://github.com/you/ocean-theme",
  "theme": { "accent": "#3ec6ff", "base": "#05070d", "surface": "#0c1220" }
}
```

Plugins can also change colors, either in their manifest with the `theme` permission or from code with `Rustify.Theme.set({ ... })`. Plugin colors are applied on top of the active theme. Colors are `#rrggbb` or `#rrggbbaa`, and `radius` is a number. Turning a theme or plugin off restores the previous look.

Rustify warns when `text` and `base` don't have enough contrast to read comfortably.

| Key | Used for |
| --- | --- |
| `base` | Window background |
| `surface` | Panels |
| `surface-raised` | Cards, menus |
| `surface-high` | Raised controls |
| `tile` | Tiles and shortcuts |
| `tile-hover` | Hovered tiles |
| `hover` | Hovered rows |
| `press` | Pressed rows |
| `text` | Primary text |
| `text-dim` | Secondary text |
| `text-faint` | Hints and timestamps |
| `accent` | Accent color (Spotify green by default) |
| `accent-hover` | Hovered accent |
| `danger` | Destructive actions and errors |
| `track` | Slider and progress tracks |
| `radius` | Corner rounding of panels, cards and dialogs, from `0` to `24` (default `8`) |

## Installing and sharing

Users can install an add-on in four ways, all from the Plugins & themes page:

- **Install from file**: pick a `.zip`.
- **From GitHub**: paste a repository link such as `https://github.com/you/hello-world`, or a folder inside one such as `https://github.com/you/addons/tree/main/hello-world`. Rustify downloads the branch as a zip from GitHub.
- **Drag and drop**: drop a `.zip` or a folder onto the Rustify window.
- **Copy the folder** into the plugins folder by hand.

To publish, put the add-on in a public GitHub repository with `manifest.json` at the top level, or in a folder you link to directly. Installing a newer version over an existing one replaces it and keeps the user's settings.

Rustify only unpacks `.json`, `.js`, `.png`, `.jpg`, `.md`, `.txt` and `LICENSE` files. An archive that contains a program (`.exe`, `.dll`, `.bat`, `.ps1` and similar) is refused. Archives are limited to 5 MB, 8 MB unpacked and 300 files, paths can't leave the add-on folder, and the add-on is fully checked before anything is moved into place.

## Safety check

Every add-on is reviewed when it's installed or changed. The result is shown on its page and in the prompt before it's turned on.

| Finding | Level |
| --- | --- |
| Runs code built from text (`eval`, `new Function`, code strings in timers) | Potentially harmful |
| Heavily encoded text or large encoded blobs | Potentially harmful |
| Decodes hidden text (`atob`, many `fromCharCode` calls) | Review |
| Minified code that can't be read | Review |
| Tries to reach the internet (`fetch`, `XMLHttpRequest`, `WebSocket`) | Review |
| A loop that never ends on its own (`while (true)`, `for (;;)`) | Review |
| Reads the user's profile (`/v1/me`) | Review |
| Changes playback from timers without user input | Review |
| Asks for every permission | Review |
| Theme text that is hard to read | Review |
| No source link | Note |

Plugins with findings appear under **Needs attention**. The prompt for a potentially harmful plugin is shown in red and lists every finding. Keep your source readable and unminified, and link your repository, to get a clean result.

## Porting from Spicetify

Rustify provides a `Spicetify` global that covers the commonly used extension APIs, so many extensions run with small changes.

| Spicetify | Rustify support |
| --- | --- |
| `Spicetify.Player` (`data`, `play`, `pause`, `togglePlay`, `next`, `back`, `seek`, `setVolume`, `getVolume`, `isPlaying`, `getProgress`, `getDuration`, `toggleShuffle`, `toggleRepeat`, `getShuffle`, `getRepeat`, `getMute`, `toggleHeart`, `addEventListener`) | Supported. `data` has the Spicetify shape (`item.uri`, `item.name`, `item.metadata`, `isPaused`, ...). |
| `Spicetify.showNotification` | Supported |
| `Spicetify.LocalStorage` | Supported, backed by plugin storage |
| `new Spicetify.ContextMenu.Item(name, onClick, shouldAdd)` | Supported. `onClick` and `shouldAdd` receive `[uri]`. |
| `new Spicetify.Topbar.Button(label, icon, onClick)` | Supported |
| `new Spicetify.Playbar.Button(label, icon, onClick)` | Supported |
| `Spicetify.CosmosAsync.get(url)` | Supported for `https://api.spotify.com/v1/...` URLs |
| `Spicetify.URI` | `isTrack`, `isAlbum`, `isArtist`, `isPlaylistV1OrV2` |
| `Spicetify.Platform.History.push` | Supported for Spotify URIs |
| `Spicetify.SVGIcons` | Rustify's built-in icon paths |
| `Spicetify.React`, `ReactDOM`, `document`, CSS injection | Not available |
| `CosmosAsync.post/put/del`, `Platform` internals | Not available |

[`examples/plugins/spicetify-song-info`](../examples/plugins/spicetify-song-info) is a Spicetify extension that runs unchanged. It uses the usual wait loop, `Topbar.Button`, `ContextMenu.Item`, `LocalStorage`, `CosmosAsync.get` and `URI`, and only adds a `manifest.json`. Rustify's tests run it on every build.

Typical port:

1. Keep or remove the wait loop that polls for `Spicetify.Player`. Everything is ready when your script runs, so it finishes immediately.
2. Replace React components and DOM code with a `Rustify.UI.Page` or a `Rustify.UI.NowPlaying.Section` and blocks. Replace CSS that hides parts of Spotify with `Rustify.UI.hide`.
3. Replace CSS themes with a manifest `theme`.
4. Add a `manifest.json` with the permissions the extension needs.

## Limits and sandbox

Plugins can only reach what the `Rustify` API exposes. They have no file system, network, process or DOM access, and they cannot see other plugins.

| Limit | Value |
| --- | --- |
| Memory | 32 MB per plugin |
| Time per callback | 250 ms, 2 s for the first run |
| Stuck callbacks before the plugin is turned off | 3 |
| UI updates | 120 per second |
| Spotify requests | 30 per minute |
| Notifications | 1 every 2 seconds |
| Timers | 100 active |
| Storage | 1 MB |
| Buttons | 4 in the top bar, 4 in the player bar |
| Menu items | 8 |
| Pages | 4 |
| Now-playing and Home sections | 3 of each, with up to 100 blocks each |
| Track row buttons | 2 per plugin, 4 in total |

Each plugin runs in its own engine instance on a background thread, so a slow plugin cannot freeze the interface. A plugin that exceeds its time budget three times is stopped and marked as failed in Settings.

## Debugging

Turn on **Developer mode** under Settings, Plugins and themes. Each plugin's page then shows a console with its `console` output, errors with stack traces, and messages from Rustify, such as rejected calls or exceeded limits.

Save a file and the plugin reloads within a couple of seconds.
