const settings = {
  notify: Rustify.Storage.get("notify") !== "false",
  hideFriends: Rustify.Storage.get("hide-friends") === "true",
};

let plays = Number(Rustify.Storage.get("plays") || 0);
let topTracks = [];

const page = Rustify.UI.Page("stats", "Listening stats", "bolt");
const panel = Rustify.UI.NowPlaying.Section("stats", "Listening stats");
const home = Rustify.UI.Home.Section("marked", "Your marked songs");

function marked() {
  return JSON.parse(Rustify.Storage.get("marked") || "[]");
}

function toggleMark(uri) {
  const list = marked();
  const index = list.indexOf(uri);
  if (index === -1) list.push(uri);
  else list.splice(index, 1);
  Rustify.Storage.set("marked", JSON.stringify(list.slice(-500)));
  return index === -1;
}

function render() {
  const track = Rustify.Player.data.track;
  page.render([
    { type: "heading", text: "Right now" },
    track
      ? { type: "row", id: "current", text: track.name, sub: track.artists.map(a => a.name).join(", "), image: track.image }
      : { type: "text", text: "Nothing is playing." },
    { type: "text", text: `Songs started since you installed this plugin: ${plays}` },
    { type: "divider" },
    { type: "toggle", id: "notify", text: "Notify on song change", sub: "Shows a toast with the song title", checked: settings.notify },
    { type: "toggle", id: "hide-friends", text: "Hide the friend activity button", sub: "Shows how plugins can change Rustify's own interface", checked: settings.hideFriends },
    { type: "button", id: "refresh", text: "Load my top tracks", primary: true },
    ...topTracks.map(t => ({ type: "row", id: "track:" + t.uri, text: t.name, sub: t.artists, image: t.image })),
  ]);
  panel.render([
    { type: "text", text: `${plays} songs started`, sub: `${marked().length} marked` },
    { type: "button", id: "open", text: "Open stats" },
  ]);
  home.render([
    { type: "text", text: `${marked().length} ${marked().length === 1 ? "song" : "songs"} marked`, sub: "Use the star on any song in a list, or in the player bar" },
    { type: "button", id: "open", text: "Open stats" },
  ]);
}

function applyHide() {
  if (settings.hideFriends) Rustify.UI.hide("friends-button");
  else Rustify.UI.show("friends-button");
}

page.onAction((id, value) => {
  if (id === "notify") {
    settings.notify = value === true;
    Rustify.Storage.set("notify", String(settings.notify));
    render();
  } else if (id === "hide-friends") {
    settings.hideFriends = value === true;
    Rustify.Storage.set("hide-friends", String(settings.hideFriends));
    applyHide();
    render();
  } else if (id === "refresh") {
    Rustify.Spotify.get("/v1/me/top/tracks?limit=5&time_range=short_term")
      .then(data => {
        topTracks = (data.items || []).map(t => ({
          uri: t.uri,
          name: t.name,
          artists: t.artists.map(a => a.name).join(", "),
          image: (t.album.images[t.album.images.length - 1] || {}).url || "",
        }));
        render();
      })
      .catch(e => Rustify.showNotification("Couldn't load top tracks: " + e.message));
  } else if (id.startsWith("track:")) {
    Rustify.navigate(id.slice("track:".length));
  }
});

panel.onAction(id => {
  if (id === "open") page.open();
});

home.onAction(id => {
  if (id === "open") page.open();
});

Rustify.UI.Topbar.Button("Listening stats", "bolt", () => page.open());

const marker = Rustify.UI.Playbar.Button("Mark this song", "star", button => {
  const track = Rustify.Player.data.track;
  if (!track) return;
  button.active = toggleMark(track.uri);
  render();
});

Rustify.UI.TrackRow.Button("Mark song", "star", (button, track) => {
  const now = toggleMark(track.uri);
  Rustify.showNotification(`${now ? "Marked" : "Unmarked"} ${track.title}`);
  syncMarker();
  render();
});

function syncMarker() {
  const track = Rustify.Player.data.track;
  marker.active = !!track && marked().includes(track.uri);
}

Rustify.UI.ContextMenu.Item("Copy song info to log", ["track"], target => {
  console.log(`${target.title} (${target.uri})`);
  Rustify.showNotification("Logged " + target.title);
});

Rustify.Player.addEventListener("songchange", data => {
  if (!data || !data.track) return;
  plays += 1;
  Rustify.Storage.set("plays", String(plays));
  if (settings.notify) Rustify.showNotification("Now playing " + data.track.name);
  syncMarker();
  render();
});

applyHide();
render();
console.log("Now Playing Tools loaded");
