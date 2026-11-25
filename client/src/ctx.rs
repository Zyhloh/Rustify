use crate::{App, CardData, Panels, Player, Shell, Store, api::Api, pages::{self, Route}, config::Config, images::Images, models, state};
use futures_util::StreamExt;
use serde_json::{Value, json};
use slint::{ComponentHandle, Image, Model, Weak};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::runtime::Handle;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
pub struct Ctx {
    pub rt: Handle,
    pub api: Api,
    pub images: Arc<Images>,
    pub app: Weak<App>,
    ws_url: String,
    volume: tokio::sync::watch::Sender<Option<(u8, bool)>>,
    search_gen: Arc<std::sync::atomic::AtomicU64>,
    config: Arc<std::sync::Mutex<Config>>,
    started: Arc<std::sync::atomic::AtomicBool>,
    decoded: Arc<std::sync::Mutex<Vec<(String, crate::images::Decoded)>>>,
}

impl Ctx {
    pub fn new(rt: Handle, config: Config, app: Weak<App>) -> Self {
        let api = Api::new(config.http_url());
        let (volume, mut pending) = tokio::sync::watch::channel(None::<(u8, bool)>);
        let worker = api.clone();
        rt.spawn(async move {
            while pending.changed().await.is_ok() {
                let Some((percent, commit)) = *pending.borrow_and_update() else { continue };
                let body = json!({ "percent": percent, "commit": commit });
                if let Err(e) = worker.post("/player/volume", body).await {
                    tracing::warn!("volume: {e}");
                }
            }
        });
        Self {
            rt,
            api,
            images: Arc::new(Images::new()),
            app,
            ws_url: config.ws_url(),
            config: Arc::new(std::sync::Mutex::new(config)),
            started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            volume,
            search_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            decoded: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn set_volume(&self, level: f32, commit: bool) {
        state::touch_volume();
        let _ = self.volume.send(Some(((level * 100.0).round().clamp(0.0, 100.0) as u8, commit)));
    }

    pub fn spawn(&self, fut: impl Future<Output = ()> + Send + 'static) {
        self.rt.spawn(fut);
    }

    pub fn ui(&self, f: impl FnOnce(&App) + Send + 'static) {
        let _ = self.app.upgrade_in_event_loop(move |app| f(&app));
    }

    pub fn command(&self, path: &'static str, body: Value) {
        let api = self.api.clone();
        self.spawn(async move {
            if let Err(e) = api.post(path, body).await {
                tracing::warn!("{path}: {e}");
            }
        });
    }

    pub fn load_images(&self, keys: Vec<String>) {
        let mut seen = std::collections::HashSet::new();
        for key in keys.into_iter().filter(|k| !k.is_empty() && !state::is_cached(k) && seen.insert(k.clone())) {
            let ctx = self.clone();
            self.spawn(async move {
                if let Some(decoded) = ctx.images.fetch(&key).await {
                    ctx.queue_image(key, decoded);
                }
            });
        }
    }

    fn queue_image(&self, key: String, decoded: crate::images::Decoded) {
        let first = {
            let Ok(mut pending) = self.decoded.lock() else { return };
            pending.push((key, decoded));
            pending.len() == 1
        };
        if !first {
            return;
        }
        let ctx = self.clone();
        self.spawn(async move {
            tokio::time::sleep(Duration::from_millis(32)).await;
            let batch = ctx.decoded.lock().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
            ctx.ui(move |app| {
                for (key, decoded) in batch {
                    state::apply_image(app, &key, Image::from_rgba8(decoded.pixels), decoded.accent);
                }
            });
        });
    }

    pub fn ui_then_images(&self, f: impl FnOnce(&App) -> Vec<String> + Send + 'static) {
        let ctx = self.clone();
        self.ui(move |app| {
            let keys = f(app);
            ctx.load_images(keys);
        });
    }

    pub async fn boot(self) {
        loop {
            match self.api.get::<models::AuthStatus>("/auth/status").await {
                Err(_) => self.ui(|app| {
                    let store = app.global::<Store>();
                    store.set_api_online(false);
                    store.set_logged_in(false);
                    store.set_status("Starting the Rustify API…".into());
                }),
                Ok(s) if !s.logged_in => self.ui(|app| {
                    let store = app.global::<Store>();
                    store.set_api_online(true);
                    store.set_logged_in(false);
                    store.set_status("Log in with your Spotify Premium account to continue.".into());
                }),
                Ok(_) => break,
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }

        self.ui(|app| {
            let store = app.global::<Store>();
            store.set_api_online(true);
            store.set_logged_in(true);
        });
        if !self.started.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.spawn(self.clone().events());
            self.spawn(self.clone().watch_status());
            self.spawn(self.clone().watch_elsewhere());
        }
        let client_id = self.config.lock().map(|c| c.client_id.clone()).unwrap_or_default();
        self.ui(move |app| {
            app.global::<Store>().set_client_id(client_id.into());
            app.global::<Store>().set_cache_size(crate::prefs::human_size(crate::prefs::cache_size()).into());
        });
        let ctx = self.clone();
        self.ui(move |app| ctx.push_audio(app));
        self.load().await;
    }

    pub async fn load(&self) {
        self.ui(|app| {
            let store = app.global::<Store>();
            store.set_loading(true);
            store.set_load_error("".into());
        });
        let (mut need_me, mut need_library, mut need_home) = (true, true, true);
        let mut attempt: u32 = 0;

        loop {
            let (me, library, home) = tokio::join!(
                async { if need_me { Some(self.api.get::<models::Me>("/me").await) } else { None } },
                async { if need_library { Some(self.api.get::<Vec<models::Card>>("/library").await) } else { None } },
                async { if need_home { Some(self.api.get::<models::Home>("/home").await) } else { None } },
            );

            match me {
                Some(Ok(me)) => {
                    need_me = false;
                    self.ui_then_images(move |app| state::set_user(app, &me));
                }
                Some(Err(e)) => tracing::warn!("me: {e}"),
                None => {}
            }
            match library {
                Some(Ok(items)) => {
                    need_library = false;
                    self.ui_then_images(move |app| state::set_library(app, &items));
                }
                Some(Err(e)) => tracing::warn!("library: {e}"),
                None => {}
            }
            match home {
                Some(Ok(home)) => {
                    need_home = false;
                    self.ui_then_images(move |app| state::set_home(app, &home));
                }
                Some(Err(e)) => tracing::warn!("home: {e}"),
                None => {}
            }

            if !(need_me || need_library || need_home) {
                self.ui(|app| app.global::<Store>().set_load_error("".into()));
                self.refresh_options().await;
                break;
            }
            attempt += 1;
            if attempt >= 3 {
                self.ui(|app| {
                    let store = app.global::<Store>();
                    store.set_load_error("Spotify isn't responding right now. Rustify will keep trying.".into());
                    store.set_loading(false);
                });
            }
            tokio::time::sleep(Duration::from_secs((2u64 << attempt.min(4)).min(30))).await;
        }
    }

    async fn watch_status(self) {
        loop {
            if let Ok(status) = self.api.get::<models::AuthStatus>("/auth/status").await {
                self.ui(move |app| {
                    let store = app.global::<Store>();
                    store.set_player_status(status.player_status.into());
                    store.set_library_missing(status.logged_in && !status.library_ready);
                    store.set_library_connecting(status.library_connecting);
                });
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn events(self) {
        loop {
            let url = format!("{}?key={}", self.ws_url, self.api.key());
            if let Ok((mut socket, _)) = tokio_tungstenite::connect_async(&url).await {
                while let Some(Ok(msg)) = socket.next().await {
                    let Message::Text(text) = msg else { continue };
                    let Ok(player) = serde_json::from_str::<models::PlayerState>(&text) else {
                        continue;
                    };
                    let ctx = self.clone();
                    self.ui(move |app| {
                        crate::plugins::player_state(&player);
                        if let Some(change) = state::apply_player(app, &player) {
                            crate::menu::track_changed(app, &ctx, &change.track_id);
                            ctx.track_changed(app, &change.track_id);
                            ctx.load_images(change.images);
                            ctx.load_artist(change.artist_id);
                            ctx.load_saved(change.track_id);
                        }
                    });
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    fn load_saved(&self, id: String) {
        if id.is_empty() {
            return;
        }
        let ctx = self.clone();
        self.spawn(async move {
            match ctx.api.get::<models::SavedState>(&format!("/saved/{id}")).await {
                Ok(saved) => ctx.ui(move |app| pages::set_saved(app, &id, saved)),
                Err(e) => tracing::warn!("saved: {e}"),
            }
        });
    }

    pub fn save_pressed(&self, app: &App) {
        let player = app.global::<Player>();
        let id = player.get_track_id().to_string();
        if id.is_empty() {
            return;
        }
        if !player.get_saved() {
            player.set_saved(true);
            self.update_saved(id, json!({ "liked": true }));
            return;
        }
        let ctx = self.clone();
        self.spawn(async move {
            match ctx.api.get::<Vec<models::PlaylistOption>>("/playlists/editable").await {
                Ok(options) => ctx.ui_then_images(move |app| pages::open_picker(app, options)),
                Err(e) => tracing::warn!("playlists: {e}"),
            }
        });
    }

    pub fn update_saved(&self, id: String, body: Value) {
        let ctx = self.clone();
        self.spawn(async move {
            match ctx.api.post_json::<models::SavedState>(&format!("/saved/{id}"), body).await {
                Ok(saved) => {
                    ctx.ui(move |app| pages::set_saved(app, &id, saved));
                    if let Ok(route @ Route::Collection { .. }) = ctx.ui_value(|_| pages::current()).await
                        && matches!(&route, Route::Collection { kind, .. } if kind == "playlist") {
                            ctx.load_route(route);
                        }
                }
                Err(e) => {
                    tracing::warn!("save: {e}");
                    ctx.ui(|app| crate::menu::toast(app, "Couldn't save that change. Try again in a moment"));
                }
            }
            ctx.reload_library().await;
        });
    }

    pub fn picker_done(&self, app: &App) {
        let id = app.global::<Player>().get_track_id().to_string();
        let diff = pages::picker_diff();
        pages::close_picker(app);
        let mut body = json!({ "add": diff.add, "remove": diff.remove });
        if let Some(liked) = diff.liked {
            body["liked"] = json!(liked);
        }
        self.update_saved(id, body);
    }

    pub fn picker_create(&self, app: &App) {
        let id = app.global::<Player>().get_track_id().to_string();
        let name = pages::next_playlist_name();
        pages::close_picker(app);
        self.create_playlist(id, name);
    }

    pub fn create_playlist(&self, track_id: String, name: String) {
        let ctx = self.clone();
        self.spawn(async move {
            let body = json!({ "name": name, "track": track_id });
            if let Err(e) = ctx.api.post_json::<Value>("/playlists", body).await {
                tracing::warn!("create playlist: {e}");
            }
            ctx.load_saved(track_id);
            ctx.reload_library().await;
        });
    }

    pub fn new_playlist(&self, app: &App) {
        let name = pages::next_playlist_name();
        let ctx = self.clone();
        let weak = app.as_weak();
        self.spawn(async move {
            match ctx.api.post_json::<models::PlaylistOption>("/playlists", json!({ "name": name })).await {
                Ok(created) => {
                    let id = created.id.clone();
                    let _ = weak.upgrade_in_event_loop({
                        let ctx = ctx.clone();
                        move |app| ctx.navigate(&app, Route::Collection { kind: "playlist".into(), id })
                    });
                    ctx.reload_library().await;
                }
                Err(e) => {
                    tracing::warn!("new playlist: {e}");
                    let busy = e.to_string().contains("429");
                    let _ = weak.upgrade_in_event_loop(move |app| {
                        crate::menu::toast(&app, if busy { "Spotify is limiting requests right now. Try again later" } else { "Couldn't create playlist" })
                    });
                }
            }
        });
    }

    pub fn navigate(&self, app: &App, route: Route) {
        let route = pages::push(app, route);
        self.load_route(route);
    }

    pub fn toggle_follow(&self, app: &App, id: String) {
        if id.is_empty() {
            return;
        }
        let follow = !pages::is_following(app, &id);
        pages::set_following(app, &id, follow);
        crate::menu::toast(app, if follow { "Added to Your Library" } else { "Removed from Your Library" });
        let ctx = self.clone();
        self.spawn(async move {
            if let Err(e) = ctx.api.post(&format!("/follow/{id}"), json!({ "follow": follow })).await {
                tracing::warn!("follow: {e}");
            }
            ctx.reload_library().await;
        });
    }

    pub async fn reload_library(&self) {
        if let Ok(items) = self.api.get::<Vec<models::Card>>("/library").await {
            self.ui_then_images(move |app| state::set_library(app, &items));
        }
        self.refresh_options().await;
    }

    async fn refresh_options(&self) {
        if let Ok(options) = self.api.get::<Vec<models::PlaylistOption>>("/playlists/editable").await {
            self.ui(move |_| pages::set_options(options));
        }
    }

    pub fn open_card(&self, app: &App, card: &CardData) {
        let route = match card.kind.as_str() {
            "playlist" | "album" => Route::Collection { kind: card.kind.to_string(), id: card.id.to_string() },
            "collection" => Route::Collection { kind: "collection".into(), id: "liked".into() },
            "artist" => Route::Artist { id: card.id.to_string() },
            "show" => Route::Show { id: card.id.to_string() },
            "episodes" => Route::Show { id: crate::podcast::YOUR_EPISODES.into() },
            "dj" => {
                app.global::<Store>().invoke_play_card(card.clone());
                return;
            }
            _ => {
                self.play(&card.kind, &card.uri);
                return;
            }
        };
        self.navigate(app, route);
    }

    async fn watch_elsewhere(self) {
        let mut tick: u64 = 0;
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            tick += 1;
            let remote = self.ui_value(|app| !app.global::<Store>().get_playing_elsewhere().is_empty()).await.unwrap_or(false);
            if !remote && !tick.is_multiple_of(6) {
                continue;
            }
            if tick.is_multiple_of(6) {
                let ctx = self.clone();
                self.ui(move |app| {
                    if app.global::<Shell>().get_right_panel() == "friends" {
                        ctx.refresh_panel(app);
                    }
                });
            }
            let mut local = false;
            let ready = self.ui_value(|app| {
                let store = app.global::<Store>();
                let playing = app.global::<Player>().get_playing() && store.get_playing_elsewhere().is_empty();
                (playing, store.get_logged_in())
            });
            if let Ok((playing, logged_in)) = ready.await {
                local = playing;
                if !logged_in {
                    continue;
                }
            }
            let name = if local {
                String::new()
            } else {
                self.api
                    .get::<serde_json::Value>("/active-device")
                    .await
                    .ok()
                    .and_then(|v| v.get("elsewhere").and_then(|e| e.as_str()).map(str::to_owned))
                    .unwrap_or_default()
            };
            self.ui(move |app| app.global::<Store>().set_playing_elsewhere(name.into()));
        }
    }

    pub fn ui_value<T: Send + 'static>(&self, f: impl FnOnce(&App) -> T + Send + 'static) -> tokio::sync::oneshot::Receiver<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.app.upgrade_in_event_loop(move |app| {
            let _ = tx.send(f(&app));
        });
        rx
    }

    pub fn open_profile(&self, app: &App, user: String) {
        self.navigate(app, Route::Profile { user });
    }

    pub fn save_client_id(&self, app: &App, id: String) {
        let store = app.global::<Store>();
        let result = self.config.lock().map_err(|_| anyhow::anyhow!("config unavailable")).and_then(|mut c| c.save_client_id(&id));
        match result {
            Ok(()) => {
                store.set_setup_error("".into());
                store.set_client_id(id.trim().into());
                let first_run = store.get_needs_setup();
                store.set_needs_setup(false);
                let ctx = self.clone();
                self.spawn(async move {
                    if !first_run {
                        let _ = ctx.api.post("/auth/logout", json!({})).await;
                        crate::sidecar::stop();
                        tokio::time::sleep(Duration::from_millis(400)).await;
                    }
                    let config = ctx.config.lock().map(|c| c.clone()).ok();
                    if let Some(config) = config {
                        if crate::sidecar::api_running(&config) && !first_run {
                            ctx.ui(|app| {
                                app.global::<Store>().set_setup_error(
                                    "Saved. Restart Rustify's service (or Rustify) to use the new app.".into(),
                                )
                            });
                        }
                        crate::sidecar::ensure_api(&config);
                    }
                    ctx.boot().await;
                });
            }
            Err(e) => store.set_setup_error(e.to_string().into()),
        }
    }

    pub fn logout(&self, app: &App) {
        let store = app.global::<Store>();
        store.set_logged_in(false);
        store.set_status("Logging out…".into());
        crate::search::clear_recent();
        let ctx = self.clone();
        self.spawn(async move {
            if let Err(e) = ctx.api.post("/auth/logout", json!({})).await {
                tracing::warn!("logout: {e}");
            }
            ctx.ui(|app| {
                let store = app.global::<Store>();
                store.set_playing_elsewhere("".into());
                pages::push(app, Route::Home);
            });
            ctx.boot().await;
        });
    }

    pub fn push_audio(&self, app: &App) {
        let body = crate::prefs::audio_body(app);
        let ctx = self.clone();
        self.spawn(async move {
            if let Err(e) = ctx.api.post("/player/audio", body).await {
                tracing::warn!("audio settings: {e}");
            }
        });
    }

    pub fn connect_library(&self) {
        let ctx = self.clone();
        self.spawn(async move {
            if let Err(e) = ctx.api.post("/auth/library", json!({})).await {
                tracing::warn!("connect library: {e}");
            }
        });
    }

    pub fn clear_cache(&self, app: &App) {
        crate::prefs::clear_image_cache();
        app.global::<Store>().set_cache_size(crate::prefs::human_size(crate::prefs::cache_size()).into());
        crate::menu::toast(app, "Cache cleared");
    }

    pub fn transfer_here(&self, app: &App) {
        let ctx = self.clone();
        let weak = app.as_weak();
        self.spawn(async move {
            let devices = ctx.api.get::<Vec<crate::panels::Device>>("/devices").await.unwrap_or_default();
            if let Some(own) = devices.iter().find(|d| d.current) {
                let _ = ctx.api.post("/devices/transfer", json!({ "id": own.id, "play": true })).await;
            }
            let _ = weak.upgrade_in_event_loop(|app| app.global::<Store>().set_playing_elsewhere("".into()));
        });
    }

    fn track_changed(&self, app: &App, track_id: &str) {
        if matches!(app.global::<Shell>().get_right_panel().as_str(), "queue" | "nowplaying") {
            self.refresh_panel(app);
        }
        let wants_lyrics = app.global::<Store>().get_page() == "lyrics" || (app.global::<Shell>().get_fullscreen() && app.global::<Shell>().get_fullscreen_lyrics());
        if wants_lyrics && crate::panels::lyrics_track() != track_id {
            self.load_lyrics(app, track_id.to_owned());
        }
    }

    pub fn open_panel(&self, app: &App, name: &str) {
        let shell = app.global::<Shell>();
        if shell.get_right_panel() == name {
            shell.set_right_panel("".into());
            return;
        }
        shell.set_right_panel(name.into());
        self.refresh_panel(app);
    }

    pub fn refresh_panel(&self, app: &App) {
        let panels = app.global::<Panels>();
        let ctx = self.clone();
        match app.global::<Shell>().get_right_panel().as_str() {
            "queue" if panels.get_queue_tab() == "recent" => {
                panels.set_queue_loading(true);
                self.spawn(async move {
                    match ctx.api.get::<Vec<models::TrackRow>>("/recent").await {
                        Ok(list) => ctx.ui_then_images(move |app| crate::panels::set_recent(app, &list)),
                        Err(e) => tracing::warn!("recent: {e}"),
                    }
                });
            }
            "queue" => {
                panels.set_queue_loading(true);
                self.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    match ctx.api.get::<crate::panels::Queue>("/queue").await {
                        Ok(queue) => ctx.ui_then_images(move |app| crate::panels::set_queue(app, &queue)),
                        Err(e) => tracing::warn!("queue: {e}"),
                    }
                });
            }
            "nowplaying" => {
                self.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    if let Ok(queue) = ctx.api.get::<crate::panels::Queue>("/queue").await {
                        ctx.ui_then_images(move |app| crate::panels::set_queue(app, &queue));
                    }
                });
            }
            "devices" => {
                panels.set_devices_loading(true);
                self.spawn(async move {
                    match ctx.api.get::<Vec<crate::panels::Device>>("/devices").await {
                        Ok(list) => ctx.ui(move |app| crate::panels::set_devices(app, &list)),
                        Err(e) => tracing::warn!("devices: {e}"),
                    }
                });
            }
            "friends" => {
                panels.set_friends_loading(true);
                self.spawn(async move {
                    match ctx.api.get::<Vec<crate::panels::Friend>>("/friends").await {
                        Ok(list) => ctx.ui_then_images(move |app| crate::panels::set_friends(app, &list)),
                        Err(e) => {
                            tracing::warn!("friends: {e}");
                            ctx.ui(|app| app.global::<Panels>().set_friends_loading(false));
                        }
                    }
                });
            }
            _ => {}
        }
    }

    pub fn show_credits(&self, app: &App) {
        let id = app.global::<Player>().get_track_id().to_string();
        if id.is_empty() {
            return;
        }
        let ctx = self.clone();
        self.spawn(async move {
            match ctx.api.get::<models::TrackInfo>(&format!("/track/{id}")).await {
                Ok(info) => ctx.ui(move |app| crate::menu::show_credits(app, &info)),
                Err(e) => {
                    tracing::warn!("credits: {e}");
                    ctx.ui(|app| crate::menu::toast(app, "Credits aren't available for this item"));
                }
            }
        });
    }

    pub fn toggle_lyrics(&self, app: &App) {
        if app.global::<Store>().get_page() == "lyrics" {
            if let Some(route) = pages::back(app) {
                self.load_route(route);
            }
            return;
        }
        self.navigate(app, Route::Lyrics);
        let track = app.global::<Player>().get_track_id().to_string();
        if !track.is_empty() {
            self.load_lyrics(app, track);
        }
    }

    pub fn load_lyrics(&self, app: &App, track_id: String) {
        crate::panels::begin_lyrics(app, &track_id);
        let ctx = self.clone();
        self.spawn(async move {
            let lyrics = ctx.api.get::<crate::panels::Lyrics>(&format!("/lyrics/{track_id}")).await.ok();
            ctx.ui(move |app| crate::panels::set_lyrics(app, &track_id, lyrics));
        });
    }

    pub fn transfer(&self, app: &App, device: String) {
        let ctx = self.clone();
        let weak = app.as_weak();
        self.spawn(async move {
            if let Err(e) = ctx.api.post("/devices/transfer", json!({ "id": device, "play": true })).await {
                tracing::warn!("transfer: {e}");
            }
            tokio::time::sleep(Duration::from_millis(600)).await;
            let _ = weak.upgrade_in_event_loop(move |app| ctx.refresh_panel(&app));
        });
    }

    pub fn search_focus(&self, app: &App) {
        if app.global::<Store>().get_page() != "search" {
            self.navigate(app, Route::Search);
        }
        if !crate::search::has_tiles() {
            let ctx = self.clone();
            self.spawn(async move {
                match ctx.api.get::<Vec<crate::search::Tile>>("/browse").await {
                    Ok(tiles) => ctx.ui_then_images(move |_| crate::search::set_tiles(&tiles)),
                    Err(e) => tracing::warn!("browse: {e}"),
                }
            });
        }
    }

    pub fn run_search(&self, app: &App, delay_ms: u64) {
        use std::sync::atomic::Ordering;
        let store = app.global::<Store>();
        let query = store.get_search_query().trim().to_string();
        let filter = store.get_search_filter().to_string();
        let generation = self.search_gen.fetch_add(1, Ordering::SeqCst) + 1;
        crate::plugins::search_changed(&query, &filter);
        if query.is_empty() {
            crate::search::clear_results(app);
            return;
        }
        store.set_search_loading(true);
        let ctx = self.clone();
        let latest = self.search_gen.clone();
        self.spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if latest.load(Ordering::SeqCst) != generation {
                return;
            }
            let path = format!(
                "/search?q={}&filter={}",
                url_encode(&query),
                if filter == "all" { "" } else { filter.as_str() }
            );
            let result = ctx.api.get::<crate::search::Results>(&path).await;
            if latest.load(Ordering::SeqCst) != generation {
                return;
            }
            match result {
                Ok(mut results) => {
                    results.filter = filter;
                    ctx.ui_then_images(move |app| crate::search::set_results(app, &results));
                }
                Err(e) => {
                    tracing::warn!("search: {e}");
                    ctx.ui(|app| {
                        let store = app.global::<Store>();
                        store.set_search_loading(false);
                        store.set_search_empty(true);
                    });
                }
            }
        });
    }

    pub fn remember(&self, card: &CardData) {
        self.load_images(crate::search::remember(card));
    }

    pub fn play_context(&self, context: &str, offset: &str) {
        self.command("/player/play", json!({ "context_uri": context, "offset_uri": offset }));
    }

    pub fn play_uris(&self, uris: Vec<String>, offset: &str) {
        if uris.is_empty() {
            return;
        }
        self.command("/player/play", json!({ "uris": uris, "offset_uri": offset }));
    }

    pub fn save_item(&self, app: &App, uri: String, saved: bool) {
        crate::menu::toast(app, if saved { "Added to Your Library" } else { "Removed from Your Library" });
        let ctx = self.clone();
        self.spawn(async move {
            if let Err(e) = ctx.api.post("/library/save", json!({ "uri": uri, "saved": saved })).await {
                tracing::warn!("library save: {e}");
            }
            ctx.reload_library().await;
        });
    }

    pub fn save_episode(&self, app: &App, uri: String, saved: bool) {
        crate::menu::toast(app, if saved { "Added to Your Episodes" } else { "Removed from Your Episodes" });
        let ctx = self.clone();
        self.spawn(async move {
            if let Err(e) = ctx.api.post("/library/save", json!({ "uri": uri, "saved": saved })).await {
                tracing::warn!("episode save: {e}");
                return;
            }
            let mine = ctx.ui_value(|app| {
                let store = app.global::<Store>();
                store.get_page() == "show" && store.get_show().kind == "episodes"
            });
            if mine.await.unwrap_or(false) {
                ctx.load_route(Route::Show { id: crate::podcast::YOUR_EPISODES.into() });
            }
        });
    }

    pub fn load_route(&self, route: Route) {
        if route == Route::News {
            let ctx = self.clone();
            self.spawn(async move {
                match ctx.api.get::<Vec<crate::news::Release>>("/whats-new").await {
                    Ok(list) => ctx.ui_then_images(move |app| crate::news::set(app, &list)),
                    Err(e) => {
                        tracing::warn!("whats new: {e}");
                        ctx.ui(|app| app.global::<Store>().set_news_loading(false));
                    }
                }
            });
            return;
        }
        if let Route::Show { id } = route.clone() {
            let ctx = self.clone();
            let path = if id == crate::podcast::YOUR_EPISODES { "/episodes".to_owned() } else { format!("/show/{}", url_encode(&id)) };
            self.spawn(async move {
                match ctx.api.get::<models::Show>(&path).await {
                    Ok(show) => ctx.ui_then_images(move |app| crate::podcast::set_show(app, &show)),
                    Err(e) => {
                        tracing::warn!("show: {e}");
                        ctx.ui(|app| app.global::<Store>().set_show_loading(false));
                    }
                }
            });
            return;
        }
        if let Route::Category { key } = route.clone() {
            let ctx = self.clone();
            self.spawn(async move {
                match ctx.api.get::<crate::search::Category>(&format!("/category/{key}")).await {
                    Ok(category) => ctx.ui_then_images(move |app| crate::search::set_category(app, &key, &category)),
                    Err(e) => {
                        tracing::warn!("category: {e}");
                        ctx.ui(|app| app.global::<Store>().set_category_loading(false));
                    }
                }
            });
            return;
        }
        if let Route::Profile { user } = route.clone() {
            let ctx = self.clone();
            self.spawn(async move {
                match ctx.api.get::<models::Profile>(&format!("/profile/{}", url_encode(&user))).await {
                    Ok(profile) => ctx.ui_then_images(move |app| crate::account::set_profile(app, &profile)),
                    Err(e) => {
                        tracing::warn!("profile: {e}");
                        ctx.ui(|app| app.global::<Store>().set_profile_loading(false));
                    }
                }
            });
            return;
        }
        if matches!(route, Route::History) {
            let ctx = self.clone();
            self.spawn(async move {
                match ctx.api.get::<Vec<models::TrackRow>>("/recent").await {
                    Ok(list) => ctx.ui_then_images(move |app| crate::account::set_history(app, &list)),
                    Err(e) => {
                        tracing::warn!("history: {e}");
                        ctx.ui(|app| app.global::<Store>().set_history_loading(false));
                    }
                }
            });
            return;
        }
        if matches!(route, Route::Search | Route::Home | Route::Lyrics | Route::Settings | Route::ShowAll { .. } | Route::Plugin { .. } | Route::Extensions | Route::Extension { .. }) {
            return;
        }
        if let Route::Artist { id } = route.clone() {
            let ctx = self.clone();
            self.spawn(async move {
                match ctx.api.get::<models::ArtistPage>(&format!("/artist-page/{id}")).await {
                    Ok(page) => ctx.ui_then_images(move |app| pages::set_artist(app, &route, &page)),
                    Err(e) => {
                        tracing::warn!("artist page: {e}");
                        ctx.ui(|app| app.global::<Store>().set_artist_loading(false));
                    }
                }
            });
            return;
        }
        let Route::Collection { kind, id } = route.clone() else { return };
        let ctx = self.clone();
        self.spawn(async move {
            self_load_collection(ctx, route, kind, id, 0).await;
        });
    }

    fn load_artist(&self, id: String) {
        if id.is_empty() {
            self.ui(|app| app.global::<Store>().set_artist(Default::default()));
            return;
        }
        let ctx = self.clone();
        self.spawn(async move {
            match ctx.api.get::<models::ArtistInfo>(&format!("/artist/{id}")).await {
                Ok(artist) => ctx.ui_then_images(move |app| state::set_artist(app, &artist)),
                Err(e) => {
                    tracing::warn!("artist: {e}");
                    ctx.ui(|app| app.global::<Store>().set_artist(Default::default()));
                }
            }
        });
    }

    pub fn play_in_context(&self, context: &str, track_uri: &str) {
        self.command("/player/play", json!({ "context_uri": context, "offset_uri": track_uri }));
    }

    pub fn play(&self, kind: &str, uri: &str) {
        let body = match kind {
            "collection" => json!({ "context_uri": "collection" }),
            "track" | "episode" => json!({ "uris": [uri] }),
            _ => json!({ "context_uri": uri }),
        };
        self.command("/player/play", body);
    }
}

pub fn url_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

async fn self_load_collection(ctx: Ctx, route: Route, kind: String, id: String, attempt: u32) {
    let path = format!("/collection/{kind}/{id}");
    ctx.ui(|app| {
        let store = app.global::<Store>();
        store.set_collection_error(false);
        if store.get_tracks().row_count() == 0 {
            store.set_collection_loading(true);
        }
    });
    match ctx.api.get_slow::<models::Collection>(&path).await {
        Ok(page) => {
            let partial = page.partial;
            let shown = route.clone();
            ctx.ui_then_images(move |app| pages::set_collection(app, &shown, &page));
            if partial && attempt < 6 {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if ctx.ui_value(|_| pages::current()).await.ok().as_ref() == Some(&route) {
                    Box::pin(self_load_collection(ctx, route, kind, id, attempt + 1)).await;
                }
            }
        }
        Err(e) => {
            tracing::warn!("collection: {e}");
            ctx.ui(|app| {
                let store = app.global::<Store>();
                store.set_collection_loading(false);
                if store.get_tracks().row_count() == 0 {
                    store.set_collection_error(true);
                }
            });
        }
    }
}
