use crate::ids::{Id, User};
use crate::{
    auth::Auth,
    collection::{self, Collection, PlaylistIndex, PlaylistOption, SavedState},
    config::Config,
    home,
    models::{ArtistInfo, AuthStatus, Card, Home, Me, image, s},
    player::Engine,
    web::WebApi,
};
use axum::{
    Json, Router,
    extract::{Path, State, WebSocketUpgrade, ws::Message},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

const CACHE_TTL: Duration = Duration::from_secs(300);

pub struct AppState {
    pub config: Config,
    pub auth: Arc<Auth>,
    pub web: WebApi,
    pub engine: Arc<Engine>,
    booting: AtomicBool,
    me: RwLock<Option<Me>>,
    home: RwLock<Option<(Instant, Home)>>,
    library: RwLock<Option<(Instant, Vec<Card>)>>,
    collections: RwLock<std::collections::HashMap<String, (Instant, Collection)>>,
    index: PlaylistIndex,
    tiles: RwLock<Option<(Instant, Vec<crate::search::Tile>)>>,
    releases: RwLock<Option<(Instant, Vec<home::Release>)>>,
    liked_ids: RwLock<Option<(Instant, std::collections::HashSet<String>)>>,
    searches: RwLock<std::collections::HashMap<String, (Instant, crate::search::SearchResults)>>,
    me_turn: tokio::sync::Mutex<()>,
    library_turn: tokio::sync::Mutex<()>,
    refreshing: std::sync::Mutex<std::collections::HashSet<&'static str>>,
    indexed_at: RwLock<Option<Instant>>,
}

const STALE_LIMIT: Duration = Duration::from_secs(24 * 3600);

fn disk_path(name: &str) -> std::path::PathBuf {
    crate::config::Config::data_dir().join(format!("{name}.json"))
}

fn disk_load<T: serde::de::DeserializeOwned>(name: &str) -> Option<(Instant, T)> {
    let raw = std::fs::read(disk_path(name)).ok()?;
    let value: Value = serde_json::from_slice(&raw).ok()?;
    let saved = value.get("saved_at")?.as_u64()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let age = Duration::from_secs(now.saturating_sub(saved));
    if age > STALE_LIMIT {
        return None;
    }
    let data = serde_json::from_value(value.get("data")?.clone()).ok()?;
    Some((Instant::now().checked_sub(age).unwrap_or_else(Instant::now), data))
}

fn disk_save<T: serde::Serialize>(name: &str, data: &T) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(raw) = serde_json::to_vec(&json!({ "saved_at": now, "data": data })) {
        let _ = std::fs::write(disk_path(name), raw);
    }
}

type Shared = State<Arc<AppState>>;

impl AppState {
    pub fn new(config: Config, auth: Arc<Auth>) -> Arc<Self> {
        let web = WebApi::new(auth.clone());
        let engine = Engine::new(config.player.initial_volume, auth.clone(), web.clone());
        Arc::new(Self {
            web,
            config,
            auth,
            engine,
            booting: AtomicBool::new(false),
            me: RwLock::new(None),
            home: RwLock::new(disk_load("home")),
            library: RwLock::new(disk_load("library")),
            collections: RwLock::new(Default::default()),
            index: PlaylistIndex::default(),
            tiles: RwLock::new(None),
            releases: RwLock::new(None),
            liked_ids: RwLock::new(None),
            searches: RwLock::new(Default::default()),
            me_turn: tokio::sync::Mutex::new(()),
            library_turn: tokio::sync::Mutex::new(()),
            refreshing: std::sync::Mutex::new(Default::default()),
            indexed_at: RwLock::new(None),
        })
    }

    async fn library_stale(&self) {
        *self.library.write().await = None;
    }

    fn begin_refresh(&self, name: &'static str) -> bool {
        self.refreshing.lock().map(|mut set| set.insert(name)).unwrap_or(false)
    }

    fn end_refresh(&self, name: &'static str) {
        if let Ok(mut set) = self.refreshing.lock() {
            set.remove(name);
        }
    }

    pub async fn boot(self: Arc<Self>, interactive: bool) {
        if self.booting.swap(true, Ordering::SeqCst) {
            return;
        }
        let state = self.clone();
        if let Err(e) = tokio::spawn(async move { state.run_boot(interactive).await }).await {
            tracing::error!("boot panicked: {e}");
            self.engine.set_error(Some("playback crashed while starting".into())).await;
        }
        self.booting.store(false, Ordering::SeqCst);
    }

    async fn run_boot(self: &Arc<Self>, interactive: bool) {
        if !self.auth.has_session().await {
            if !interactive {
                return;
            }
            if let Err(e) = self.auth.login().await {
                tracing::error!("login failed: {e:#}");
                return;
            }
        }
        let state = self.clone();
        tokio::spawn(async move { state.refresh_index().await });

        if let Err(e) = self.engine.start_player(&self.config).await {
            tracing::error!("web player failed to start: {e:#}");
            self.engine.set_error(Some(format!("{e:#}"))).await;
        }

        if !self.engine.has_credentials() && !interactive {
            tracing::warn!("no saved library session; Spotify-made playlists and pins need one");
            return;
        }
        if let Err(e) = self.engine.start_session(&self.config.spotify.redirect_uri, interactive).await {
            tracing::warn!("library session: {e:#}");
        }
    }

    async fn save(&self, uri: &str, saved: bool) -> anyhow::Result<()> {
        let web = match uri.strip_prefix("spotify:track:") {
            Some(id) => collection::set_liked(&self.web, id, saved).await,
            None => collection::set_library_item(&self.web, uri, saved).await,
        };
        let Err(web_err) = web else { return Ok(()) };
        let Some(session) = self.engine.session().await else { return Err(web_err) };
        crate::native::collection_write(&session, uri, saved)
            .await
            .inspect_err(|e| tracing::warn!("save {uri} via web: {web_err:#}; native: {e:#}"))?;
        self.collections.write().await.remove("collection:liked");
        *self.liked_ids.write().await = None;
        Ok(())
    }

    async fn liked_natively(&self, id: &str) -> bool {
        if let Some((at, ids)) = self.liked_ids.read().await.as_ref()
            && at.elapsed() < Duration::from_secs(60) {
                return ids.contains(id);
            }
        let Some(session) = self.engine.session().await else { return false };
        let Ok(all) = crate::native::collection_checked(&session, "collection").await else { return false };
        let ids: std::collections::HashSet<String> = all
            .into_iter()
            .filter_map(|u| u.strip_prefix("spotify:track:").map(str::to_owned))
            .collect();
        let found = ids.contains(id);
        *self.liked_ids.write().await = Some((Instant::now(), ids));
        found
    }

    async fn in_library(&self, uri: &str) -> bool {
        match collection::in_library(&self.web, uri).await {
            Ok(saved) => saved,
            Err(_) => self.library.read().await.as_ref().is_some_and(|(_, items)| items.iter().any(|c| c.uri == uri)),
        }
    }

    async fn refresh_index_if_stale(&self) {
        let fresh = self.indexed_at.read().await.is_some_and(|at| at.elapsed() < Duration::from_secs(300));
        if !fresh {
            self.refresh_index().await;
        }
    }

    async fn refresh_index(&self) {
        *self.indexed_at.write().await = Some(Instant::now());
        let me = match self.me().await {
            Ok(me) => me,
            Err(e) => return tracing::warn!("playlist index: {e:#}"),
        };
        let Err(e) = self.index.refresh(&self.web, &me).await else { return };
        match self.engine.session_or_wait(Duration::from_secs(15)).await {
            Some(session) => {
                let owned: Vec<collection::PlaylistOption> = crate::native::rootlist(&session)
                    .await
                    .into_iter()
                    .filter(|r| r.owner == me.id && !r.name.is_empty())
                    .map(|r| collection::PlaylistOption {
                        id: r.uri.rsplit(':').next().unwrap_or_default().to_owned(),
                        uri: r.uri,
                        name: r.name,
                        image: r.image,
                        total: 0,
                    })
                    .collect();
                tracing::info!("playlist index via library session: {} editable playlists", owned.len());
                self.index.set_options(owned).await;
            }
            None => tracing::warn!("playlist index: {e:#}"),
        }
    }

    async fn me(&self) -> anyhow::Result<Me> {
        if let Some(me) = self.me.read().await.clone() {
            return Ok(me);
        }
        let _turn = self.me_turn.lock().await;
        if let Some(me) = self.me.read().await.clone() {
            return Ok(me);
        }
        let saved = crate::config::Config::data_dir().join("me.json");
        let me = match home::me(&self.web).await {
            Ok(me) => {
                if let Ok(raw) = serde_json::to_vec(&me) {
                    let _ = std::fs::write(&saved, raw);
                }
                me
            }
            Err(web_err) => {
                let cached = std::fs::read(&saved).ok().and_then(|raw| serde_json::from_slice::<Me>(&raw).ok());
                match cached {
                    Some(me) => me,
                    None => match self.engine.session_or_wait(Duration::from_secs(10)).await {
                        Some(session) => crate::extras::native_me(&session).await.map_err(|e| web_err.context(e))?,
                        None => return Err(web_err),
                    },
                }
            }
        };
        *self.me.write().await = Some(me.clone());
        Ok(me)
    }
}

pub struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let message = format!("{:#}", self.0);
        let status = if message.contains("not logged in") {
            StatusCode::UNAUTHORIZED
        } else if message.contains("rate limited") {
            StatusCode::TOO_MANY_REQUESTS
        } else if message.starts_with("invalid") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(|| async { Json(json!({ "status": "ok" })) }))
        .route("/auth/status", get(auth_status))
        .route("/auth/login", post(auth_login))
        .route("/auth/logout", post(auth_logout))
        .route("/auth/library", post(auth_library))
        .route("/plugin/spotify", get(plugin_spotify))
        .route("/plugin/audio", post(plugin_audio))
        .route("/me", get(me))
        .route("/home", get(home_feed))
        .route("/library", get(library))
        .route("/artist/{id}", get(artist))
        .route("/collection/{kind}/{id}", get(collection_page))
        .route("/artist-page/{id}", get(artist_page))
        .route("/follow/{id}", post(follow))
        .route("/track/{id}", get(track_info))
        .route("/player/queue", post(player_queue))
        .route("/saved/{id}", get(saved).post(update_saved))
        .route("/playlists/editable", get(editable_playlists))
        .route("/playlists", post(create_playlist))
        .route("/player", get(player_state))
        .route("/player/play", post(player_play))
        .route("/player/toggle", post(player_toggle))
        .route("/player/pause", post(player_pause))
        .route("/player/next", post(player_next))
        .route("/player/prev", post(player_prev))
        .route("/player/seek", post(player_seek))
        .route("/player/volume", post(player_volume))
        .route("/player/shuffle", post(player_shuffle))
        .route("/player/repeat", post(player_repeat))
        .route("/player/audio", post(player_audio))
        .route("/events", get(events))
        .route("/webplayer", get(webplayer_page))
        .route("/search", get(search))
        .route("/browse", get(browse))
        .route("/category/{key}", get(category))
        .route("/queue", get(queue))
        .route("/recent", get(recent))
        .route("/devices", get(devices))
        .route("/active-device", get(active_device))
        .route("/devices/transfer", post(transfer))
        .route("/lyrics/{id}", get(lyrics))
        .route("/friends", get(friends))
        .route("/profile/{user}", get(profile))
        .route("/show/{id}", get(show))
        .route("/episodes", get(episodes))
        .route("/whats-new", get(whats_new))
        .route("/playlist/{id}/edit", post(edit_playlist))
        .route("/playlist/{id}/delete", post(delete_playlist))
        .route("/library/save", post(save_to_library))
        .with_state(state)
}

async fn auth_status(State(st): Shared) -> Json<AuthStatus> {
    Json(AuthStatus {
        logged_in: st.auth.has_session().await,
        player_ready: st.engine.is_ready().await,
        player_error: st.engine.error().await,
        player_status: st.engine.status().await.into(),
        library_ready: st.engine.session().await.is_some(),
        library_connecting: st.engine.connecting.load(Ordering::SeqCst),
    })
}

#[derive(Deserialize)]
struct PluginQuery {
    path: String,
}

fn plugin_path_ok(path: &str) -> bool {
    path.starts_with("/v1/")
        && path.len() <= 512
        && !path.contains("..")
        && !path.contains("//")
        && path.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-.?=&,%:+".contains(&b))
}

#[derive(Deserialize)]
struct PluginAudioBody {
    plugin: String,
    action: String,
    #[serde(default)]
    value: f64,
    #[serde(default)]
    track: Option<crate::player::ExternalTrack>,
}

pub fn public_https(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    let Some(host) = parsed.host_str() else { return false };
    parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && url.len() <= 2000
        && host.contains('.')
        && host.parse::<std::net::IpAddr>().is_err()
        && !host.trim_start_matches('[').starts_with(|c: char| c.is_ascii_digit() || c == ':')
        && !host.eq_ignore_ascii_case("localhost")
        && !host.to_ascii_lowercase().ends_with(".localhost")
        && !host.to_ascii_lowercase().ends_with(".local")
}

fn plugin_id_ok(id: &str) -> bool {
    (3..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

async fn plugin_audio(State(st): Shared, Json(b): Json<PluginAudioBody>) -> ApiResult<Value> {
    if !plugin_id_ok(&b.plugin) {
        return Err(anyhow::anyhow!("invalid plugin").into());
    }
    if b.action == "load" {
        let Some(mut track) = b.track else { return Err(anyhow::anyhow!("missing track").into()) };
        if !public_https(&track.url) {
            return Err(anyhow::anyhow!("audio must come from a public https address").into());
        }
        if !track.image.is_empty() && !public_https(&track.image) {
            track.image.clear();
        }
        let clip = |t: &mut String, n: usize| *t = t.chars().filter(|c| !c.is_control()).take(n).collect();
        clip(&mut track.title, 200);
        clip(&mut track.artist, 200);
        clip(&mut track.album, 200);
        clip(&mut track.id, 100);
        st.engine.load_external(&b.plugin, track).await?;
    } else {
        st.engine.external_command(&b.plugin, &b.action, b.value).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn plugin_spotify(State(st): Shared, axum::extract::Query(q): axum::extract::Query<PluginQuery>) -> ApiResult<Value> {
    if !plugin_path_ok(&q.path) {
        return Err(anyhow::anyhow!("invalid plugin request").into());
    }
    let path = q.path.trim_start_matches("/v1");
    Ok(Json(st.web.get::<Value>(path).await?))
}

async fn auth_library(State(st): Shared) -> Json<Value> {
    if st.auth.has_session().await {
        tokio::spawn(async move {
            if let Err(e) = st.engine.start_session(&st.config.spotify.redirect_uri, true).await {
                tracing::warn!("library session: {e:#}");
            }
        });
    }
    Json(json!({ "started": true }))
}

async fn auth_login(State(st): Shared) -> Json<Value> {
    tokio::spawn(st.clone().boot(true));
    Json(json!({ "started": true }))
}

async fn auth_logout(State(st): Shared) -> Json<Value> {
    st.engine.shutdown().await;
    st.auth.logout().await;
    *st.me.write().await = None;
    *st.home.write().await = None;
    *st.library.write().await = None;
    st.collections.write().await.clear();
    st.searches.write().await.clear();
    *st.releases.write().await = None;
    *st.liked_ids.write().await = None;
    collection::forget_tracks().await;
    st.index.forget().await;
    for name in ["me", "home", "library"] {
        let _ = std::fs::remove_file(disk_path(name));
    }
    tracing::info!("logged out");
    Json(json!({ "ok": true }))
}

async fn me(State(st): Shared) -> ApiResult<Me> {
    Ok(Json(st.me().await?))
}

async fn home_feed(State(st): Shared) -> ApiResult<Home> {
    let cached = st.home.read().await.clone();
    if let Some((at, home)) = cached {
        if at.elapsed() >= CACHE_TTL && st.begin_refresh("home") {
            let state = st.clone();
            tokio::spawn(async move {
                let _ = build_home(&state).await;
                state.end_refresh("home");
            });
        }
        return Ok(Json(home));
    }
    Ok(Json(build_home(&st).await?))
}

async fn build_home(st: &Arc<AppState>) -> anyhow::Result<Home> {
    let me = st.me().await?;
    let session = st.engine.session_or_wait(Duration::from_secs(5)).await;
    let complete = session.is_some();
    let home = home::home(&st.web, &me, session).await;
    if complete && !home.shortcuts.is_empty() {
        disk_save("home", &home);
        *st.home.write().await = Some((Instant::now(), home.clone()));
    }
    Ok(home)
}

async fn library(State(st): Shared) -> ApiResult<Vec<Card>> {
    let cached = st.library.read().await.clone();
    if let Some((at, items)) = cached {
        if at.elapsed() >= CACHE_TTL && st.begin_refresh("library") {
            let state = st.clone();
            tokio::spawn(async move {
                let _ = build_library(&state).await;
                state.end_refresh("library");
            });
        }
        return Ok(Json(items));
    }
    Ok(Json(build_library(&st).await?))
}

async fn build_library(st: &Arc<AppState>) -> anyhow::Result<Vec<Card>> {
    let started = Instant::now();
    let _turn = st.library_turn.lock().await;
    if let Some((at, items)) = st.library.read().await.as_ref()
        && *at >= started {
            return Ok(items.clone());
        }
    let me = st.me().await?;
    let session = st.engine.session_or_wait(Duration::from_secs(10)).await;
    let complete = session.is_some();
    let (recents, pins, episodes, root) = match session.clone() {
        Some(session) => {
            let (recents, pins, episodes, root) = tokio::join!(
                crate::native::recently_played(&session),
                crate::native::pins(&session),
                st.web.get::<Value>("/me/episodes?limit=1"),
                crate::native::rootlist(&session),
            );
            let count = match episodes.ok().and_then(|v| v.get("total").and_then(Value::as_u64)) {
                Some(count) => count,
                None => crate::native::collection_set(&session, "listenlater").await.len() as u64,
            };
            (recents, pins, count, root)
        }
        None => (Default::default(), Vec::new(), 0, Vec::new()),
    };
    let items: Vec<Card> = home::library(&st.web, session.as_ref(), &me, recents, pins, episodes, root)
        .await
        .into_iter()
        .map(Card::retag)
        .collect();
    if complete && !items.is_empty() {
        disk_save("library", &items);
        *st.library.write().await = Some((Instant::now(), items.clone()));
    }
    Ok(items)
}

async fn artist(State(st): Shared, Id(id): Id) -> ApiResult<ArtistInfo> {
    let artist_path = format!("/artists/{id}");
    let artist_uri = format!("spotify:artist:{id}");
    let (artist, following) = tokio::join!(
        st.web.get::<Value>(&artist_path),
        st.in_library(&artist_uri),
    );
    let artist = artist?;
    Ok(Json(ArtistInfo {
        id: s(&artist, "id"),
        uri: s(&artist, "uri"),
        name: s(&artist, "name"),
        image: image(&artist, 300),
        followers: artist.pointer("/followers/total").and_then(Value::as_u64),
        genres: artist
            .get("genres")
            .and_then(Value::as_array)
            .map(|g| g.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
            .unwrap_or_default(),
        following,
    }))
}

async fn collection_page(State(st): Shared, Path((kind, id)): Path<(String, String)>) -> ApiResult<Collection> {
    Ok(Json(load_collection(&st, &kind, &id).await?))
}

async fn load_collection(st: &AppState, kind: &str, id: &str) -> anyhow::Result<Collection> {
    if !crate::ids::is_id(id) || !matches!(kind, "playlist" | "album" | "collection") {
        anyhow::bail!("invalid collection");
    }
    let key = format!("{kind}:{id}");
    if let Some((at, page)) = st.collections.read().await.get(&key)
        && at.elapsed() < Duration::from_secs(120) {
            return Ok(page.clone());
        }
    let me = st.me().await?;
    let page = match kind {
        "playlist" => match collection::playlist(&st.web, &me, id).await {
            Ok(page) => page,
            Err(web_err) => match st.engine.session().await {
                Some(session) => collection::playlist_native(&session, &me, id)
                    .await
                    .inspect_err(|e| tracing::warn!("playlist {id} via web: {web_err:#}; native: {e:#}"))?,
                None => return Err(web_err),
            },
        },
        "album" => {
            let mut page = collection::album(&st.web, id).await?;
            if let (Some(session), false) = (st.engine.session().await, page.owner_id.is_empty()) {
                page.more = crate::artist::releases(&session, &page.owner_id, 6)
                    .await
                    .into_iter()
                    .filter(|r| r.card.id != id)
                    .map(|r| r.card)
                    .take(10)
                    .collect();
            }
            page
        }
        _ => match collection::liked(&st.web, &me).await {
            Ok(page) => page,
            Err(web_err) => match st.engine.session().await {
                Some(session) => collection::liked_native(&session, &me)
                    .await
                    .inspect_err(|e| tracing::warn!("liked via web: {web_err:#}; native: {e:#}"))?,
                None => return Err(web_err),
            },
        },
    };
    if !page.partial {
        let mut cache = st.collections.write().await;
        cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(600));
        if cache.len() >= 24
            && let Some(oldest) = cache.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) {
                cache.remove(&oldest);
            }
        cache.insert(key, (Instant::now(), page.clone()));
    }
    Ok(page)
}

async fn saved(State(st): Shared, Id(id): Id) -> ApiResult<SavedState> {
    let liked = match collection::is_liked(&st.web, &id).await {
        Ok(liked) => liked,
        Err(_) => st.liked_natively(&id).await,
    };
    Ok(Json(SavedState { liked, playlists: st.index.containing(&id).await }))
}

#[derive(Deserialize)]
struct SavedBody {
    liked: Option<bool>,
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

async fn update_saved(State(st): Shared, Id(id): Id, Json(b): Json<SavedBody>) -> ApiResult<SavedState> {
    if let Some(liked) = b.liked {
        st.save(&format!("spotify:track:{id}"), liked).await?;
        st.collections.write().await.remove("collection:liked");
        st.library_stale().await;
    }
    let changes = b
        .add
        .iter()
        .map(|p| (p, true))
        .chain(b.remove.iter().map(|p| (p, false)))
        .filter(|(p, _)| crate::ids::is_id(p));
    let session = st.engine.session().await;
    let mut failed = None;
    for (playlist, present) in changes {
        if let Err(e) = collection::set_in_playlist(&st.web, session.as_ref(), &st.index, playlist, &id, present).await {
            tracing::warn!("playlist {playlist}: {e:#}");
            failed = Some(e);
        }
        st.collections.write().await.remove(&format!("playlist:{playlist}"));
        st.library_stale().await;
    }
    if let Some(e) = failed {
        return Err(e.into());
    }
    let Json(mut state) = saved(State(st), Id(id)).await?;
    if let Some(liked) = b.liked {
        state.liked = liked;
    }
    Ok(Json(state))
}

async fn editable_playlists(State(st): Shared) -> ApiResult<Vec<PlaylistOption>> {
    let options = st.index.options().await;
    if options.is_empty() {
        st.refresh_index().await;
        return Ok(Json(st.index.options().await));
    }
    let state = st.clone();
    tokio::spawn(async move { state.refresh_index_if_stale().await });
    Ok(Json(options))
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
    track: Option<String>,
}

async fn create_playlist(State(st): Shared, Json(b): Json<CreateBody>) -> ApiResult<PlaylistOption> {
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 100 || b.track.as_deref().is_some_and(|t| !crate::ids::is_id(t)) {
        return Err(anyhow::anyhow!("invalid playlist").into());
    }
    let me = st.me().await?;
    let mut option = collection::create_playlist(&st.web, &me, name).await?;
    st.index.insert_playlist(option.clone()).await;
    if let Some(track) = b.track {
        let session = st.engine.session().await;
        collection::set_in_playlist(&st.web, session.as_ref(), &st.index, &option.id, &track, true).await?;
        option.total = 1;
    }
    *st.library.write().await = None;
    Ok(Json(option))
}

async fn artist_page(State(st): Shared, Id(id): Id) -> ApiResult<crate::artist::ArtistPage> {
    let session = st.engine.session().await;
    let uri = format!("spotify:artist:{id}");
    let (page, following) = tokio::join!(crate::artist::page(&st.web, session, &id), st.in_library(&uri));
    let mut page = page?;
    page.following = following;
    if let Some(session) = st.engine.session().await
        && let Some((listeners, verified)) = crate::native::artist_stats(&session, &id).await {
            page.listeners = listeners;
            page.verified = verified;
        }
    Ok(Json(page))
}

#[derive(Deserialize)]
struct FollowBody {
    follow: bool,
}

async fn follow(State(st): Shared, Id(id): Id, Json(b): Json<FollowBody>) -> ApiResult<Value> {
    st.save(&format!("spotify:artist:{id}"), b.follow).await?;
    *st.library.write().await = None;
    Ok(Json(json!({ "following": b.follow })))
}

async fn track_info(State(st): Shared, Id(id): Id) -> ApiResult<crate::artist::TrackInfo> {
    Ok(Json(crate::artist::track_info(&st.web, &id).await?))
}

#[derive(Deserialize)]
struct QueueBody {
    uri: String,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    filter: String,
}

async fn search(State(st): Shared, axum::extract::Query(q): axum::extract::Query<SearchQuery>) -> ApiResult<crate::search::SearchResults> {
    let query = q.q.trim().chars().take(200).collect::<String>();
    if query.is_empty() {
        return Err(anyhow::anyhow!("empty query").into());
    }
    let key = format!("{}|{}", q.filter, query.to_lowercase());
    if let Some((at, hit)) = st.searches.read().await.get(&key)
        && at.elapsed() < Duration::from_secs(120) {
            return Ok(Json(hit.clone()));
        }
    let results = crate::search::search(&st.web, &query, &q.filter).await?;
    let mut cache = st.searches.write().await;
    if cache.len() > 40 {
        cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(120));
    }
    cache.insert(key, (Instant::now(), results.clone()));
    Ok(Json(results))
}

async fn browse(State(st): Shared) -> Json<Vec<crate::search::Tile>> {
    if let Some((at, tiles)) = st.tiles.read().await.as_ref()
        && at.elapsed() < Duration::from_secs(12 * 3600) {
            return Json(tiles.clone());
        }
    let mut tiles = crate::search::tiles(&st.web).await;
    if let Ok(Json(cards)) = library(State(st.clone())).await {
        let cover = cards.iter().find(|c| c.subtitle.contains("Made for")).and_then(|c| c.image.clone());
        if let Some(tile) = tiles.iter_mut().find(|t| t.key == "made-for-you") {
            tile.image = cover;
        }
    }
    if tiles.iter().any(|t| t.image.is_some()) {
        *st.tiles.write().await = Some((Instant::now(), tiles.clone()));
    }
    Json(tiles)
}

async fn category(State(st): Shared, Path(key): Path<String>) -> ApiResult<crate::search::Category> {
    let made_for = if key == "made-for-you" {
        library(State(st.clone()))
            .await
            .map(|Json(cards)| cards.into_iter().filter(|c| c.subtitle.contains("Made for") || c.subtitle.ends_with("Spotify")).collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(Json(crate::search::category(&st.web, &key, made_for).await?))
}

async fn queue(State(st): Shared) -> ApiResult<crate::extras::Queue> {
    let live = crate::extras::queue(&st.web).await;
    if let Ok(queue) = &live
        && queue.now.is_some() {
            return Ok(Json(live?));
        }
    let Some((Some(context), uri)) = st.engine.staged() else { return Ok(Json(live?)) };
    let (kind, id) = if context.ends_with(":collection") {
        ("collection", "liked".to_owned())
    } else {
        let mut parts = context.splitn(3, ':').skip(1);
        (parts.next().unwrap_or_default(), parts.next().unwrap_or_default().to_owned())
    };
    let Ok(page) = load_collection(&st, kind, &id).await else {
        return Ok(Json(crate::extras::Queue { now: None, next: Vec::new() }));
    };
    let tracks = page.tracks;
    let at = tracks.iter().position(|t| t.uri == uri);
    let now = at.and_then(|i| tracks.get(i).cloned());
    let next = at.map(|i| tracks.iter().skip(i + 1).take(30).cloned().collect()).unwrap_or_default();
    Ok(Json(crate::extras::Queue { now, next }))
}

async fn active_device(State(st): Shared) -> ApiResult<Value> {
    let own = st.engine.device().await.unwrap_or_default();
    let list = crate::extras::devices(&st.web).await?;
    let other = list.iter().find(|d| d.active && d.id != own);
    st.engine.set_remote(other.map(|d| d.id.clone())).await;
    Ok(Json(json!({ "elsewhere": other.map(|d| d.name.clone()).unwrap_or_default() })))
}

async fn recent(State(st): Shared) -> ApiResult<Vec<crate::collection::TrackRow>> {
    match crate::extras::recent(&st.web).await {
        Ok(list) => Ok(Json(list)),
        Err(e) => {
            let local = crate::player::read_history();
            if local.is_empty() {
                return Err(e.into());
            }
            Ok(Json(
                local
                    .into_iter()
                    .map(|t| crate::collection::TrackRow {
                        id: t.id,
                        uri: t.uri,
                        album_id: String::new(),
                        name: t.name,
                        artists: t.artists,
                        album: t.album,
                        image: t.image_small,
                        duration: crate::collection::format_duration(t.duration_ms as u64),
                        explicit: t.explicit,
                        added: String::new(),
                        added_by: String::new(),
                    })
                    .collect(),
            ))
        }
    }
}

async fn devices(State(st): Shared) -> ApiResult<Vec<crate::extras::Device>> {
    let own = st.engine.device().await.unwrap_or_default();
    let mut list = crate::extras::devices(&st.web).await?;
    for device in list.iter_mut() {
        device.current = device.id == own;
    }
    if !list.iter().any(|d| d.active)
        && let Some(device) = list.iter_mut().find(|d| d.current) {
            device.active = true;
        }
    list.sort_by_key(|d| (!d.active, !d.current));
    Ok(Json(list))
}

#[derive(Deserialize)]
struct TransferBody {
    id: String,
    #[serde(default)]
    play: bool,
}

async fn transfer(State(st): Shared, Json(b): Json<TransferBody>) -> ApiResult<Value> {
    if !crate::ids::is_id(&b.id) {
        return Err(anyhow::anyhow!("invalid device").into());
    }
    crate::extras::transfer(&st.web, &b.id, b.play).await?;
    let own = st.engine.device().await.unwrap_or_default();
    st.engine.set_remote((b.id != own).then(|| b.id.clone())).await;
    Ok(Json(json!({ "ok": true })))
}

async fn lyrics(State(st): Shared, Id(id): Id) -> ApiResult<crate::extras::Lyrics> {
    let session = st.engine.session().await.ok_or_else(|| anyhow::anyhow!("library session not ready"))?;
    Ok(Json(crate::extras::lyrics(&session, &id).await?))
}

async fn friends(State(st): Shared) -> ApiResult<Vec<crate::extras::Friend>> {
    let session = st.engine.session().await.ok_or_else(|| anyhow::anyhow!("library session not ready"))?;
    Ok(Json(crate::extras::friends(&session).await?))
}

async fn profile(State(st): Shared, User(user): User) -> ApiResult<crate::extras::Profile> {
    let me = st.me().await?;
    let session = st.engine.session().await;
    Ok(Json(crate::extras::profile(&st.web, session.as_ref(), &me, &user).await?))
}

async fn show(State(st): Shared, Id(id): Id) -> ApiResult<crate::extras::ShowPage> {
    let uri = format!("spotify:show:{id}");
    let (page, saved) = tokio::join!(crate::extras::show(&st.web, &id), st.in_library(&uri));
    let mut page = page?;
    page.saved = saved;
    if page.publisher.is_empty()
        && let Some(session) = st.engine.session().await {
            page.publisher = crate::native::show_publisher(&session, &id).await.unwrap_or_default();
        }
    Ok(Json(page))
}

async fn whats_new(State(st): Shared) -> ApiResult<Vec<home::Release>> {
    if let Some((at, list)) = st.releases.read().await.as_ref()
        && at.elapsed() < Duration::from_secs(1800) {
            return Ok(Json(list.clone()));
        }
    let session = st.engine.session_or_wait(Duration::from_secs(10)).await;
    let list = home::whats_new(&st.web, session).await;
    if !list.is_empty() {
        *st.releases.write().await = Some((Instant::now(), list.clone()));
    }
    Ok(Json(list))
}

async fn episodes(State(st): Shared) -> ApiResult<crate::extras::ShowPage> {
    Ok(Json(crate::extras::saved_episodes(&st.web).await?))
}

#[derive(Deserialize)]
struct EditBody {
    name: String,
    #[serde(default)]
    description: String,
}

async fn edit_playlist(State(st): Shared, Id(id): Id, Json(b): Json<EditBody>) -> ApiResult<Value> {
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > 100 || b.description.chars().count() > 300 {
        return Err(anyhow::anyhow!("invalid playlist details").into());
    }
    crate::extras::edit_playlist(&st.web, &id, name, b.description.trim()).await?;
    st.collections.write().await.remove(&format!("playlist:{id}"));
    *st.library.write().await = None;
    Ok(Json(json!({ "ok": true })))
}

async fn delete_playlist(State(st): Shared, Id(id): Id) -> ApiResult<Value> {
    if let Err(web_err) = crate::extras::unfollow_playlist(&st.web, &id).await {
        let Some(session) = st.engine.session().await else { return Err(web_err.into()) };
        crate::native::rootlist_remove(&session, &format!("spotify:playlist:{id}"))
            .await
            .inspect_err(|e| tracing::warn!("delete playlist {id} via web: {web_err:#}; native: {e:#}"))?;
    }
    st.index.remove_playlist(&id).await;
    st.collections.write().await.remove(&format!("playlist:{id}"));
    *st.library.write().await = None;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct SaveBody {
    uri: String,
    saved: bool,
}

async fn save_to_library(State(st): Shared, Json(b): Json<SaveBody>) -> ApiResult<Value> {
    if !crate::ids::is_uri(&b.uri, &["album", "playlist", "show", "episode", "artist", "track"]) {
        return Err(anyhow::anyhow!("invalid uri").into());
    }
    st.save(&b.uri, b.saved).await?;
    *st.library.write().await = None;
    Ok(Json(json!({ "saved": b.saved })))
}

async fn webplayer_page() -> axum::response::Html<&'static str> {
    axum::response::Html(crate::webplayer::PAGE)
}

async fn player_queue(State(st): Shared, Json(b): Json<QueueBody>) -> ApiResult<Value> {
    let uris = if crate::ids::is_uri(&b.uri, &["track", "episode"]) {
        vec![b.uri]
    } else if crate::ids::is_uri(&b.uri, &["album", "playlist"]) {
        let mut parts = b.uri.splitn(3, ':').skip(1);
        let (kind, id) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
        load_collection(&st, kind, id).await?.tracks.into_iter().map(|t| t.uri).take(25).collect()
    } else if b.uri == "collection" {
        load_collection(&st, "collection", "liked").await?.tracks.into_iter().map(|t| t.uri).take(25).collect()
    } else {
        return Err(anyhow::anyhow!("invalid uri").into());
    };
    let device = st.engine.device().await.map(|d| format!("&device_id={d}")).unwrap_or_default();
    let count = uris.len();
    for (i, uri) in uris.iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        st.web.post(&format!("/me/player/queue?uri={uri}{device}"), None).await?;
    }
    Ok(Json(json!({ "queued": count })))
}

async fn player_state(State(st): Shared) -> Json<crate::models::PlayerState> {
    Json(st.engine.current().await)
}

#[derive(Deserialize)]
struct PlayBody {
    context_uri: Option<String>,
    #[serde(default)]
    uris: Vec<String>,
    offset_uri: Option<String>,
    #[serde(default)]
    radio: bool,
}

async fn player_play(State(st): Shared, body: Option<Json<PlayBody>>) -> ApiResult<Value> {
    let (context, uris, offset, radio) = match body {
        Some(Json(b)) => (b.context_uri, b.uris, b.offset_uri, b.radio),
        None => (None, Vec::new(), None, false),
    };
    let context_ok = context
        .as_deref()
        .is_none_or(|c| c == "collection" || crate::ids::is_uri(c, &["album", "playlist", "artist", "show", "track"]) || crate::ids::is_liked_uri(c));
    let tracks_ok = uris.len() <= 800 && uris.iter().all(|u| crate::ids::is_uri(u, &["track", "episode"]));
    let offset_ok = offset.as_deref().is_none_or(|u| crate::ids::is_uri(u, &["track", "episode"]));
    if !(context_ok && tracks_ok && offset_ok) {
        return Err(anyhow::anyhow!("invalid playback request").into());
    }
    play_or_expand(&st, context, uris, offset, radio).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn context_tracks(st: &AppState, context: &str) -> Vec<String> {
    let (kind, id) = if context == "collection" || crate::ids::is_liked_uri(context) {
        ("collection", "liked".to_owned())
    } else {
        let mut parts = context.splitn(3, ':').skip(1);
        (parts.next().unwrap_or_default(), parts.next().unwrap_or_default().to_owned())
    };
    match load_collection(st, kind, &id).await {
        Ok(page) => page.tracks.into_iter().map(|t| t.uri).collect(),
        Err(_) => Vec::new(),
    }
}

async fn play_or_expand(st: &AppState, context: Option<String>, uris: Vec<String>, offset: Option<String>, radio: bool) -> anyhow::Result<()> {
    let first = st.engine.play(context.clone(), uris, offset.clone(), radio).await;
    let (Err(e), Some(context), false) = (&first, context, radio) else { return first };
    let refused = { let m = format!("{e:#}"); m.contains("403") || m.contains("404") };
    if !refused {
        return first;
    }
    tracing::warn!("context play failed, playing the list instead: {e:#}");
    let all = context_tracks(st, &context).await;
    let start = offset.as_ref().and_then(|o| all.iter().position(|u| u == o)).unwrap_or(0);
    let list: Vec<String> = all.into_iter().skip(start).take(500).collect();
    if list.is_empty() {
        return first;
    }
    let lead = list[0].clone();
    st.engine.play(None, list, Some(lead), false).await
}

async fn player_audio(State(st): Shared, Json(b): Json<crate::player::AudioSettings>) -> ApiResult<Value> {
    st.engine.set_audio(b).await;
    Ok(Json(json!({ "ok": true })))
}

async fn player_toggle(State(st): Shared) -> ApiResult<Value> {
    match st.engine.take_staged_for_toggle().await {
        Some((context, uri)) => match context {
            Some(context) => play_or_expand(&st, Some(context), Vec::new(), Some(uri), false).await?,
            None => st.engine.play(None, vec![uri], None, false).await?,
        },
        None => st.engine.toggle().await?,
    }
    Ok(Json(json!({ "ok": true })))
}

async fn player_pause(State(st): Shared) -> ApiResult<Value> {
    st.engine.pause().await?;
    Ok(Json(json!({ "ok": true })))
}

async fn player_next(State(st): Shared) -> ApiResult<Value> {
    st.engine.next().await?;
    Ok(Json(json!({ "ok": true })))
}

async fn player_prev(State(st): Shared) -> ApiResult<Value> {
    st.engine.prev().await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct SeekBody {
    position_ms: u32,
}

async fn player_seek(State(st): Shared, Json(b): Json<SeekBody>) -> ApiResult<Value> {
    st.engine.seek(b.position_ms).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct VolumeBody {
    percent: u8,
    #[serde(default = "yes")]
    commit: bool,
}

fn yes() -> bool {
    true
}

async fn player_volume(State(st): Shared, Json(b): Json<VolumeBody>) -> ApiResult<Value> {
    st.engine.volume(b.percent, b.commit).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ShuffleBody {
    state: bool,
}

async fn player_shuffle(State(st): Shared, Json(b): Json<ShuffleBody>) -> ApiResult<Value> {
    st.engine.shuffle(b.state).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct RepeatBody {
    state: String,
}

async fn player_repeat(State(st): Shared, Json(b): Json<RepeatBody>) -> ApiResult<Value> {
    st.engine.repeat(&b.state).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn events(State(st): Shared, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        let mut rx = st.engine.subscribe();
        let initial = serde_json::to_string(&st.engine.state().await).unwrap_or_default();
        if socket.send(Message::Text(initial.into())).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Ok(state) => {
                        let text = serde_json::to_string(&state).unwrap_or_default();
                        if socket.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                },
                incoming = socket.recv() => if !matches!(incoming, Some(Ok(_))) {
                    break;
                },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{plugin_path_ok, public_https};

    #[test]
    fn public_audio_addresses() {
        assert!(public_https("https://cdn.example.com/a.mp3"));
        assert!(!public_https("http://cdn.example.com/a.mp3"));
        assert!(!public_https("https://127.0.0.1/a.mp3"));
        assert!(!public_https("https://[::1]/a.mp3"));
        assert!(!public_https("https://localhost/a.mp3"));
        assert!(!public_https("https://router.local/a.mp3"));
        assert!(!public_https("https://user@cdn.example.com/a.mp3"));
        assert!(!public_https("file:///C:/a.mp3"));
    }

    #[test]
    fn plugin_paths() {
        assert!(plugin_path_ok("/v1/me/top/artists?limit=5"));
        assert!(plugin_path_ok("/v1/search?q=lucid%20dreams&type=track"));
        assert!(!plugin_path_ok("/v2/me"));
        assert!(!plugin_path_ok("/v1/../token"));
        assert!(!plugin_path_ok("https://evil.example/v1/me"));
        assert!(!plugin_path_ok("/v1//me"));
        assert!(!plugin_path_ok("/v1/me
"));
    }
}
