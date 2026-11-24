(() => {
  const host = globalThis.__host;
  const hostAsync = globalThis.__hostAsync;
  delete globalThis.__host;
  delete globalThis.__hostAsync;

  const call = (op, payload) => {
    const out = host(op, JSON.stringify(payload === undefined ? null : payload));
    return out === "" ? undefined : JSON.parse(out);
  };

  const pending = new Map();
  const callAsync = (op, payload) =>
    new Promise((resolve, reject) => {
      const id = hostAsync(op, JSON.stringify(payload === undefined ? null : payload));
      pending.set(id, { resolve, reject });
    });

  const text = (value, max) => String(value === undefined || value === null ? "" : value).slice(0, max);
  const format = args =>
    args
      .map(a => {
        if (typeof a === "string") return a;
        if (a instanceof Error) return a.stack || a.message;
        try {
          return JSON.stringify(a);
        } catch (e) {
          return String(a);
        }
      })
      .join(" ");

  const listeners = new Map();
  const on = (type, cb) => {
    if (typeof cb !== "function") return () => {};
    if (!listeners.has(type)) listeners.set(type, new Set());
    listeners.get(type).add(cb);
    return () => off(type, cb);
  };
  const off = (type, cb) => listeners.get(type)?.delete(cb);

  let state = { track: null, isPlaying: false, position: 0, duration: 0, volume: 0, shuffle: false, repeat: "off", context: null };

  const handlers = new Map();
  let nextHandle = 1;
  const handle = fn => {
    const id = String(nextHandle++);
    handlers.set(id, fn);
    return id;
  };

  const timers = new Map();
  let nextTimer = 1;
  const addTimer = (fn, ms, repeat, args) => {
    if (typeof fn !== "function") return 0;
    const id = nextTimer++;
    timers.set(id, { fn, args, repeat });
    call("timer.set", { id, ms: Math.max(0, Number(ms) || 0), repeat });
    return id;
  };
  const clearTimer = id => {
    if (timers.delete(id)) call("timer.clear", { id });
  };

  globalThis.setTimeout = (fn, ms, ...args) => addTimer(fn, ms, false, args);
  globalThis.setInterval = (fn, ms, ...args) => addTimer(fn, ms, true, args);
  globalThis.clearTimeout = clearTimer;
  globalThis.clearInterval = clearTimer;

  globalThis.console = {
    log: (...a) => call("log", { level: "info", text: format(a) }),
    info: (...a) => call("log", { level: "info", text: format(a) }),
    debug: (...a) => call("log", { level: "info", text: format(a) }),
    warn: (...a) => call("log", { level: "warn", text: format(a) }),
    error: (...a) => call("log", { level: "error", text: format(a) }),
  };

  const ICONS = {
    play: "M8 5.5 V18.5 L19 12 Z",
    pause: "M7 5 H10 V19 H7 Z M14 5 H17 V19 H14 Z",
    next: "M6 5.5 V18.5 L15 12 Z M17 5 V19",
    heart: "M12 20 L4.5 12.5 A4.5 4.5 0 0 1 12 6.3 A4.5 4.5 0 0 1 19.5 12.5 Z",
    star: "M12 3.5 L14.6 9 L20.5 9.6 L16 13.6 L17.3 19.5 L12 16.5 L6.7 19.5 L8 13.6 L3.5 9.6 L9.4 9 Z",
    list: "M4 6.5 H20 M4 12 H20 M4 17.5 H14",
    music: "M9 18 A2.5 2.5 0 1 1 4 18 A2.5 2.5 0 1 1 9 18 Z M9 18 V5.5 L19 3.5 V16 M19 16 A2.5 2.5 0 1 1 14 16 A2.5 2.5 0 1 1 19 16 Z",
    settings: "M12 8.5 A3.5 3.5 0 1 1 11.99 8.5 Z M12 2.5 V5 M12 19 V21.5 M2.5 12 H5 M19 12 H21.5 M5.3 5.3 L7.1 7.1 M16.9 16.9 L18.7 18.7 M5.3 18.7 L7.1 16.9 M16.9 7.1 L18.7 5.3",
    search: "M17.5 11 A6.5 6.5 0 1 1 4.5 11 A6.5 6.5 0 1 1 17.5 11 Z M15.9 15.9 L20.5 20.5",
    info: "M21 12 A9 9 0 1 1 3 12 A9 9 0 1 1 21 12 Z M12 11 V16.5 M12 7.5 V8",
    bolt: "M13 3 L5 13.5 H11.5 L10.5 21 L19 10.5 H12.5 Z",
    lyrics: "M5 5.5 H19 M5 10 H19 M5 14.5 H13 M17 13 V20",
    download: "M12 4 V15 M7.5 10.5 L12 15 L16.5 10.5 M5 19.5 H19",
    plus: "M12 5.5 V18.5 M5.5 12 H18.5",
  };

  const icon = value => {
    if (!value) return ICONS.music;
    const name = String(value);
    if (ICONS[name]) return ICONS[name];
    const fromSvg = name.match(/\sd="([^"]+)"/);
    return text(fromSvg ? fromSvg[1] : name, 2000);
  };

  const controls = (kind, op) => (label, iconName, onClick, active) => {
    let props = { label: text(label, 40), icon: icon(iconName), active: !!active };
    const id = handle(target => typeof onClick === "function" && onClick(button, target || undefined));
    const sync = () => call(op, { id, ...props });
    const button = {
      get label() {
        return props.label;
      },
      set label(v) {
        props.label = text(v, 40);
        sync();
      },
      get icon() {
        return props.icon;
      },
      set icon(v) {
        props.icon = icon(v);
        sync();
      },
      get active() {
        return props.active;
      },
      set active(v) {
        props.active = !!v;
        sync();
      },
      update(next) {
        if (next.label !== undefined) props.label = text(next.label, 40);
        if (next.icon !== undefined) props.icon = icon(next.icon);
        if (next.active !== undefined) props.active = !!next.active;
        sync();
      },
      remove() {
        handlers.delete(id);
        call(op, { id, remove: true });
      },
    };
    sync();
    return button;
  };

  const TYPES = ["track", "album", "playlist", "artist", "show", "episode"];

  const menuItem = (label, types, onClick) => {
    const wanted = (Array.isArray(types) && types.length ? types : TYPES).filter(t => TYPES.includes(t));
    const id = handle(target => typeof onClick === "function" && onClick(target));
    call("ui.menu", { id, label: text(label, 40), types: wanted });
    return {
      remove() {
        handlers.delete(id);
        call("ui.menu", { id, remove: true });
      },
    };
  };

  const NODE_TYPES = ["heading", "text", "button", "toggle", "slider", "input", "row", "divider", "image"];

  const normalize = (nodes, max) =>
    (Array.isArray(nodes) ? nodes : [])
      .filter(n => n && NODE_TYPES.includes(n.type))
      .slice(0, max)
      .map(n => ({
        type: n.type,
        id: text(n.id, 64),
        text: text(n.text ?? n.label ?? n.title, 2000),
        sub: text(n.sub ?? n.subtitle ?? n.placeholder, 500),
        value: typeof n.value === "number" ? n.value : 0,
        input: typeof n.value === "string" ? text(n.value, 500) : "",
        min: typeof n.min === "number" ? n.min : 0,
        max: typeof n.max === "number" ? n.max : 1,
        on: !!(n.checked ?? n.on),
        primary: !!n.primary,
        image: text(n.image ?? n.url, 500),
      }));

  const section = place => (id, title) => {
    const sectionId = text(id, 40);
    let actionHandler = null;
    const key = handle(event => actionHandler && actionHandler(event.id, event.value));
    call("ui.section", { id: sectionId, place, title: text(title, 60), handle: key });
    return {
      render(nodes) {
        call("ui.section.render", { id: sectionId, nodes: normalize(nodes, 100) });
      },
      onAction(fn) {
        actionHandler = typeof fn === "function" ? fn : null;
      },
      remove() {
        handlers.delete(key);
        call("ui.section", { id: sectionId, place, remove: true });
      },
    };
  };

  const ELEMENTS = ["friends-button", "news-button", "now-playing-button", "lyrics-button", "queue-button", "devices-button", "mini-player-button", "fullscreen-button"];

  const page = (id, title, iconName) => {
    const pageId = text(id, 40);
    let actionHandler = null;
    const key = handle(event => actionHandler && actionHandler(event.id, event.value));
    call("ui.page", { id: pageId, title: text(title, 60), icon: icon(iconName), handle: key });
    return {
      render(nodes) {
        call("ui.render", { id: pageId, nodes: normalize(nodes, 300) });
      },
      onAction(fn) {
        actionHandler = typeof fn === "function" ? fn : null;
      },
      open() {
        call("ui.open", { id: pageId });
      },
      remove() {
        handlers.delete(key);
        call("ui.page", { id: pageId, remove: true });
      },
    };
  };

  const Player = {
    get data() {
      return state;
    },
    play: () => call("player", { action: "play" }),
    pause: () => call("player", { action: "pause" }),
    togglePlay: () => call("player", { action: "toggle" }),
    next: () => call("player", { action: "next" }),
    back: () => call("player", { action: "prev" }),
    seek: ms => call("player", { action: "seek", value: Number(ms) || 0 }),
    setVolume: level => call("player", { action: "volume", value: Math.max(0, Math.min(1, Number(level) || 0)) }),
    toggleShuffle: () => call("player", { action: "shuffle" }),
    toggleRepeat: () => call("player", { action: "repeat" }),
    toggleLike: () => call("player", { action: "like" }),
    getVolume: () => state.volume,
    isPlaying: () => state.isPlaying,
    getProgress: () => state.position,
    getDuration: () => state.duration,
    addEventListener: (type, cb) => on(type, cb),
    removeEventListener: (type, cb) => off(type, cb),
  };

  const Storage = {
    get: key => call("storage.get", { key: text(key, 128) }) ?? null,
    set: (key, value) => call("storage.set", { key: text(key, 128), value: text(value, 100000) }),
    remove: key => call("storage.remove", { key: text(key, 128) }),
    keys: () => call("storage.keys", null) || [],
    clear: () => call("storage.clear", null),
  };

  const Rustify = Object.freeze({
    version: call("info", null).version,
    plugin: call("info", null).plugin,
    Player: Object.freeze(Player),
    Storage: Object.freeze(Storage),
    Spotify: Object.freeze({ get: path => callAsync("spotify.get", { path: String(path) }) }),
    UI: Object.freeze({
      Topbar: Object.freeze({ Button: controls("topbar", "ui.topbar") }),
      Playbar: Object.freeze({ Button: controls("playbar", "ui.playbar") }),
      ContextMenu: Object.freeze({ Item: menuItem }),
      Page: page,
      NowPlaying: Object.freeze({ Section: section("now-playing") }),
      Home: Object.freeze({ Section: section("home") }),
      TrackRow: Object.freeze({ Button: controls("trackrow", "ui.trackrow") }),
      hide: element => call("ui.hide", { element: text(element, 40), hidden: true }),
      show: element => call("ui.hide", { element: text(element, 40), hidden: false }),
      elements: Object.freeze([...ELEMENTS]),
      icons: Object.freeze(Object.keys(ICONS)),
    }),
    Theme: Object.freeze({
      set: colors => call("theme.set", colors || {}),
      reset: () => call("theme.reset", null),
    }),
    on,
    off,
    navigate: uri => call("navigate", { uri: text(uri, 200) }),
    showNotification: message => call("notify", { text: text(message, 200) }),
  });

  const asSpicetifyData = () => {
    const t = state.track;
    return {
      item: t && {
        uri: t.uri,
        name: t.name,
        duration: { milliseconds: t.duration },
        artists: t.artists.map(a => ({ name: a.name, uri: "spotify:artist:" + a.id })),
        album: { name: t.album, images: t.image ? [{ url: t.image }] : [] },
        metadata: {
          title: t.name,
          artist_name: t.artists.map(a => a.name).join(", "),
          album_title: t.album,
          image_url: t.image || "",
          image_xlarge_url: t.image || "",
        },
      },
      isPaused: !state.isPlaying,
      duration: state.duration,
      position: state.position,
      context: { uri: state.context || "" },
    };
  };

  const toUris = target => (target && target.uri ? [target.uri] : []);

  class SpicetifyMenuItem {
    constructor(name, onClick, shouldAdd, iconName) {
      this.name = name;
      this.onClick = onClick;
      this.shouldAdd = typeof shouldAdd === "function" ? shouldAdd : () => true;
      this.icon = iconName;
      this.entry = null;
    }
    register() {
      if (this.entry) return;
      this.entry = menuItem(this.name, TYPES, target => {
        const uris = toUris(target);
        if (this.shouldAdd(uris)) this.onClick(uris);
      });
    }
    deregister() {
      if (this.entry) this.entry.remove();
      this.entry = null;
    }
  }

  const spicetifyButton = make =>
    class {
      constructor(label, iconName, onClick, disabled, active) {
        this.inner = make(label, iconName, () => onClick && onClick(this), active);
      }
      get label() {
        return this.inner.label;
      }
      set label(v) {
        this.inner.label = v;
      }
      get icon() {
        return this.inner.icon;
      }
      set icon(v) {
        this.inner.icon = v;
      }
      get active() {
        return this.inner.active;
      }
      set active(v) {
        this.inner.active = v;
      }
      register() {}
      deregister() {
        this.inner.remove();
      }
    };

  const eventMap = { songchange: "songchange", onplaypause: "onplaypause", onprogress: "onprogress" };

  const Spicetify = {
    Player: {
      ...Player,
      get data() {
        return asSpicetifyData();
      },
      addEventListener: (type, cb) => on(eventMap[type] || type, () => cb({ data: asSpicetifyData(), type })),
      getRepeat: () => ({ off: 0, context: 1, track: 2 })[state.repeat] ?? 0,
      getShuffle: () => !!state.shuffle,
      getMute: () => state.volume === 0,
      toggleHeart: () => Player.toggleLike(),
    },
    showNotification: message => Rustify.showNotification(message),
    LocalStorage: { get: Storage.get, set: Storage.set, remove: Storage.remove, clear: Storage.clear },
    ContextMenu: { Item: SpicetifyMenuItem },
    Topbar: { Button: spicetifyButton(controls("topbar", "ui.topbar")) },
    Playbar: { Button: spicetifyButton(controls("playbar", "ui.playbar")) },
    CosmosAsync: {
      get: url => {
        const raw = String(url);
        const path = raw.startsWith("https://api.spotify.com") ? raw.slice("https://api.spotify.com".length) : raw;
        return Rustify.Spotify.get(path);
      },
    },
    URI: {
      isTrack: uri => String(uri).startsWith("spotify:track:"),
      isAlbum: uri => String(uri).startsWith("spotify:album:"),
      isArtist: uri => String(uri).startsWith("spotify:artist:"),
      isPlaylistV1OrV2: uri => String(uri).includes(":playlist:"),
    },
    Platform: { History: { push: path => Rustify.navigate(String(path && path.pathname ? path.pathname : path)) } },
    SVGIcons: Object.fromEntries(Object.entries(ICONS)),
  };

  globalThis.__emit = (type, json) => {
    const data = json ? JSON.parse(json) : null;
    if (type === "state") {
      state = data;
      return;
    }
    for (const cb of [...(listeners.get(type) || [])]) {
      try {
        cb(data);
      } catch (e) {
        console.error(e);
      }
    }
  };

  globalThis.__resolve = (id, ok, json) => {
    const entry = pending.get(id);
    if (!entry) return;
    pending.delete(id);
    if (ok) entry.resolve(json ? JSON.parse(json) : null);
    else entry.reject(new Error(json));
  };

  globalThis.__fire = id => {
    const timer = timers.get(id);
    if (!timer) return;
    if (!timer.repeat) timers.delete(id);
    timer.fn(...(timer.args || []));
  };

  globalThis.__action = (key, json) => {
    const fn = handlers.get(key);
    if (fn) fn(json ? JSON.parse(json) : null);
  };

  Object.defineProperty(globalThis, "Rustify", { value: Rustify, writable: false, configurable: false });
  Object.defineProperty(globalThis, "Spicetify", { value: Object.freeze(Spicetify), writable: false, configurable: false });
})();
