const settings = {
  notify: Rustify.Storage.get("notify") !== "false",
};

let plays = Number(Rustify.Storage.get("plays") || 0);
let topTracks = [];

const page = Rustify.UI.Page("stats", "Listening stats", "bolt");

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
    { type: "button", id: "refresh", text: "Load my top tracks", primary: true },
    ...topTracks.map(t => ({ type: "row", id: "track:" + t.uri, text: t.name, sub: t.artists, image: t.image })),
  ]);
}

page.onAction((id, value) => {
  if (id === "notify") {
    settings.notify = value === true;
    Rustify.Storage.set("notify", String(settings.notify));
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

Rustify.UI.Topbar.Button("Listening stats", "bolt", () => page.open());

const marker = Rustify.UI.Playbar.Button("Mark this song", "star", button => {
  const track = Rustify.Player.data.track;
  if (!track) return;
  const marked = JSON.parse(Rustify.Storage.get("marked") || "[]");
  const index = marked.indexOf(track.uri);
  if (index === -1) marked.push(track.uri);
  else marked.splice(index, 1);
  Rustify.Storage.set("marked", JSON.stringify(marked.slice(-500)));
  button.active = index === -1;
});

function syncMarker() {
  const track = Rustify.Player.data.track;
  const marked = JSON.parse(Rustify.Storage.get("marked") || "[]");
  marker.active = !!track && marked.includes(track.uri);
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

render();
console.log("Now Playing Tools loaded");
