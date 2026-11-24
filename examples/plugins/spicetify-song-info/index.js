(async function songInfo() {
  while (!Spicetify?.Player?.data || !Spicetify?.showNotification) {
    await new Promise(resolve => setTimeout(resolve, 100));
  }

  const { Player, LocalStorage, ContextMenu, Topbar, CosmosAsync, URI } = Spicetify;
  const KEY = "song-info:enabled";
  let enabled = LocalStorage.get(KEY) !== "false";

  new Topbar.Button(
    "Song info",
    `<svg viewBox="0 0 24 24"><path d="M12 3.5 L14.6 9 L20.5 9.6 L16 13.6 L17.3 19.5 L12 16.5 L6.7 19.5 L8 13.6 L3.5 9.6 L9.4 9 Z"/></svg>`,
    self => {
      enabled = !enabled;
      self.active = enabled;
      LocalStorage.set(KEY, String(enabled));
      Spicetify.showNotification(enabled ? "Song info turned on" : "Song info turned off");
    },
    false,
    enabled
  );

  Player.addEventListener("songchange", event => {
    const item = event.data.item;
    if (!enabled || !item) return;
    Spicetify.showNotification(`${item.metadata.title} by ${item.metadata.artist_name}`);
  });

  new ContextMenu.Item(
    "Show release date",
    async uris => {
      const id = uris[0].split(":")[2];
      try {
        const track = await CosmosAsync.get(`https://api.spotify.com/v1/tracks/${id}`);
        Spicetify.showNotification(`${track.name} came out on ${track.album.release_date}`);
      } catch (e) {
        Spicetify.showNotification("Couldn't load the release date");
      }
    },
    uris => uris.length === 1 && URI.isTrack(uris[0])
  ).register();

  console.log("Song info ready, shuffle is " + (Player.getShuffle() ? "on" : "off"));
})();
