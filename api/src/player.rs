use crate::{
    auth::Auth,
    config::Config,
    models::{ArtistRef, NowPlaying, PlayerState},
    web::WebApi,
    webplayer::WebPlayer,
};
use anyhow::anyhow;
use librespot::{
    core::{SpotifyUri, authentication::Credentials, cache::Cache, config::SessionConfig, session::Session},
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{RwLock, broadcast, mpsc};

pub struct Engine {
    web: WebApi,
    auth: Arc<Auth>,
    player: Mutex<Option<WebPlayer>>,
    device: RwLock<Option<String>>,
    session: RwLock<Option<Session>>,
    state: RwLock<PlayerState>,
    events: broadcast::Sender<PlayerState>,
    user_id: RwLock<String>,
    error: RwLock<Option<String>>,
    status: RwLock<&'static str>,
    pending: Mutex<Option<(&'static str, Instant)>>,
    staged: Mutex<Option<(Option<String>, String)>>,
    remote: RwLock<Option<String>>,
    remote_state: RwLock<Option<(Instant, PlayerState)>>,
    pub connecting: std::sync::atomic::AtomicBool,
    remote_failures: std::sync::atomic::AtomicU32,
    token_pending: std::sync::atomic::AtomicBool,
    audio: std::sync::Mutex<AudioSettings>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioSettings {
    pub eq: bool,
    pub bands: [f32; 6],
    pub level: String,
    pub mono: bool,
    pub curve: String,
    #[serde(default)]
    pub fade: u8,
    #[serde(default = "very_high")]
    pub quality: String,
    #[serde(default = "enabled")]
    pub normalize: bool,
}

fn very_high() -> String {
    "very_high".into()
}

fn enabled() -> bool {
    true
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { eq: false, bands: [0.0; 6], level: "normal".into(), mono: false, curve: "natural".into(), fade: 0, quality: very_high(), normalize: true }
    }
}

impl AudioSettings {
    pub fn sanitized(mut self) -> Self {
        for band in self.bands.iter_mut() {
            *band = if band.is_finite() { band.clamp(-12.0, 12.0) } else { 0.0 };
        }
        if !matches!(self.level.as_str(), "quiet" | "normal" | "loud") {
            self.level = "normal".into();
        }
        self.fade = self.fade.min(12);
        if self.quality != "high" {
            self.quality = very_high();
        }
        if !matches!(self.curve.as_str(), "linear" | "natural" | "gentle") {
            self.curve = "natural".into();
        }
        self
    }

    pub fn gain(&self, percent: u8) -> f32 {
        let position = percent.min(100) as f32 / 100.0;
        let exponent = match self.curve.as_str() {
            "linear" => 1.0,
            "gentle" => 3.0,
            _ => 2.0,
        };
        position.powf(exponent)
    }
}

impl Engine {
    pub fn new(volume: u8, auth: Arc<Auth>, web: WebApi) -> Arc<Self> {
        let (events, _) = broadcast::channel(64);
        let engine = Arc::new(Self {
            web,
            auth,
            player: Mutex::new(None),
            device: RwLock::new(None),
            session: RwLock::new(None),
            state: RwLock::new(PlayerState {
                volume,
                repeat: "off".into(),
                ..Default::default()
            }),
            events,
            user_id: RwLock::new(String::new()),
            error: RwLock::new(None),
            status: RwLock::new("idle"),
            pending: Mutex::new(None),
            staged: Mutex::new(None),
            remote: RwLock::new(None),
            remote_state: RwLock::new(None),
            connecting: std::sync::atomic::AtomicBool::new(false),
            remote_failures: std::sync::atomic::AtomicU32::new(0),
            token_pending: std::sync::atomic::AtomicBool::new(false),
            audio: std::sync::Mutex::new(AudioSettings::default()),
        });
        tokio::spawn(Arc::clone(&engine).mirror_remote());
        engine
    }

    async fn mirror_remote(self: Arc<Self>) {
        let mut last: Option<(String, bool, u32)> = None;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if self.remote().await.is_none() {
                last = None;
                continue;
            }
            let state = self.current().await;
            if self.remote().await.is_none() {
                let _ = self.events.send(self.state().await);
                continue;
            }
            let key = (state.track.as_ref().map(|t| t.uri.clone()).unwrap_or_default(), state.is_playing, state.position_ms / 5000);
            if last.as_ref() != Some(&key) || state.is_playing {
                last = Some(key);
                let _ = self.events.send(state);
            }
        }
    }

    pub async fn set_remote(&self, device: Option<String>) {
        let mut remote = self.remote.write().await;
        if *remote != device {
            *remote = device;
            *self.remote_state.write().await = None;
        }
    }

    async fn remote(&self) -> Option<String> {
        self.remote.read().await.clone()
    }

    pub async fn current(&self) -> PlayerState {
        let Some(device) = self.remote().await else { return self.state().await };
        if let Some((at, state)) = self.remote_state.read().await.as_ref()
            && at.elapsed() < Duration::from_millis(4500) {
                return state.clone();
            }
        let v = match self.web.get::<Value>("/me/player?additional_types=episode").await {
            Ok(v) => v,
            Err(e) => {
                let failures = self.remote_failures.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                if failures >= 3 {
                    tracing::warn!("stopped following the other device: {e:#}");
                    self.set_remote(None).await;
                }
                return self.state().await;
            }
        };
        self.remote_failures.store(0, std::sync::atomic::Ordering::SeqCst);
        let active = v.pointer("/device/id").and_then(Value::as_str).unwrap_or_default();
        if v.is_null() || active != device {
            self.set_remote(None).await;
            return self.state().await;
        }
        let local = self.state().await;
        let state = PlayerState {
            ready: local.ready,
            track: v.get("item").filter(|t| !t.is_null()).map(now_playing).or(local.track),
            is_playing: v.get("is_playing").and_then(Value::as_bool).unwrap_or(false),
            position_ms: v.get("progress_ms").and_then(Value::as_u64).unwrap_or(0) as u32,
            shuffle: v.get("shuffle_state").and_then(Value::as_bool).unwrap_or(false),
            repeat: v.get("repeat_state").and_then(Value::as_str).unwrap_or("off").to_owned(),
            volume: v.pointer("/device/volume_percent").and_then(Value::as_u64).map(|p| p.min(100) as u8).unwrap_or(local.volume),
            context: v.pointer("/context/uri").and_then(Value::as_str).map(str::to_owned),
        };
        *self.remote_state.write().await = Some((Instant::now(), state.clone()));
        state
    }

    async fn remote_command(&self, method: &str, path: &str, body: Option<Value>) -> Option<anyhow::Result<()>> {
        let device = self.remote().await?;
        let sep = if path.contains('?') { '&' } else { '?' };
        let url = format!("{path}{sep}device_id={device}");
        let result = match method {
            "POST" => self.web.post(&url, body).await.map(|_| ()),
            _ => self.web.put(&url, body).await,
        };
        *self.remote_state.write().await = None;
        Some(result)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlayerState> {
        self.events.subscribe()
    }

    pub async fn state(&self) -> PlayerState {
        self.state.read().await.clone()
    }

    pub async fn is_ready(&self) -> bool {
        self.device.read().await.is_some()
    }

    pub async fn device(&self) -> Option<String> {
        self.device.read().await.clone()
    }

    pub async fn error(&self) -> Option<String> {
        self.error.read().await.clone()
    }

    pub async fn set_error(&self, error: Option<String>) {
        if error.is_some() {
            *self.status.write().await = "error";
        }
        *self.error.write().await = error;
    }

    pub fn has_credentials(&self) -> bool {
        let dir = Config::data_dir();
        dir.join("credentials.bin").exists() || dir.join("credentials.json").exists()
    }

    fn sealed_credentials() -> Option<Credentials> {
        let raw = std::fs::read(Config::data_dir().join("credentials.bin")).ok()?;
        serde_json::from_slice(&crate::vault::open(&raw)?).ok()
    }

    fn seal_session(session: &Session) {
        use librespot::protocol::authentication::AuthenticationType;
        let dir = Config::data_dir();
        let credentials = Credentials {
            username: Some(session.username()),
            auth_type: AuthenticationType::AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS,
            auth_data: session.auth_data(),
        };
        let sealed = serde_json::to_vec(&credentials).ok().and_then(|raw| crate::vault::seal(&raw));
        if let Some(sealed) = sealed
            && std::fs::write(dir.join("credentials.bin"), sealed).is_ok() {
                let _ = std::fs::remove_file(dir.join("credentials.json"));
            }
    }

    pub async fn status(&self) -> &'static str {
        *self.status.read().await
    }

    pub async fn session(&self) -> Option<Session> {
        self.session.read().await.clone()
    }

    pub async fn session_or_wait(&self, timeout: Duration) -> Option<Session> {
        let start = Instant::now();
        loop {
            if let Some(session) = self.session().await {
                return Some(session);
            }
            if !self.has_credentials() || start.elapsed() >= timeout {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn shutdown(&self) {
        self.call("disconnect", &[]);
        *self.device.write().await = None;
        if let Some(session) = self.session.write().await.take() {
            session.shutdown();
        }
        *self.user_id.write().await = String::new();
        *self.status.write().await = "idle";
        *self.error.write().await = None;
        let volume = self.state.read().await.volume;
        self.update(|s| {
            *s = PlayerState { volume, repeat: "off".into(), ..Default::default() };
        })
        .await;
        let _ = std::fs::remove_file(Config::data_dir().join("credentials.json"));
        let _ = std::fs::remove_file(Config::data_dir().join("credentials.bin"));
        let _ = std::fs::remove_file(Config::data_dir().join("last.json"));
        let _ = std::fs::remove_file(Config::data_dir().join("history.json"));
        let _ = std::fs::remove_file(Config::data_dir().join("tracks.json"));
    }

    pub async fn start_player(self: &Arc<Self>, config: &Config) -> anyhow::Result<()> {
        if self.player.lock().map(|p| p.is_some()).unwrap_or(true) {
            if self.device().await.is_none() {
                *self.status.write().await = "connecting";
                self.call("connect", &[]);
            }
            return Ok(());
        }
        *self.status.write().await = "connecting";

        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("name", &config.player.device_name)
            .append_pair("volume", &(config.player.initial_volume as f32 / 100.0).to_string())
            .finish();
        let url = format!("http://{}:{}/webplayer?{query}", config.api.host, config.api.port);

        let (tx, rx) = mpsc::unbounded_channel();
        let player = WebPlayer::spawn(url, Config::data_dir().join("webview"), tx)?;
        if let Ok(mut slot) = self.player.lock() {
            *slot = Some(player);
        }
        tokio::spawn(Arc::clone(self).listen(rx));
        tracing::info!("web player starting");
        Ok(())
    }

    pub async fn start_session(&self, redirect_uri: &str, interactive: bool) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;
        if self.session.read().await.is_some() || self.connecting.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = self.connect_session(redirect_uri, interactive).await;
        self.connecting.store(false, Ordering::SeqCst);
        result
    }

    async fn connect_session(&self, redirect_uri: &str, interactive: bool) -> anyhow::Result<()> {
        let dir = Config::data_dir();
        let legacy = Cache::new(Some(dir.clone()), None, None, None)?;
        let cache = Cache::new(None, Some(dir.clone()), None, None)?;
        let session_config = SessionConfig::default();
        let credentials = match Self::sealed_credentials().or_else(|| legacy.credentials()) {
            Some(credentials) => credentials,
            None if interactive => {
                tracing::info!("library session login required, opening browser");
                let token = crate::oauth::authorize(&session_config.client_id, redirect_uri, &["streaming"]).await?;
                Credentials::with_access_token(token.access_token)
            }
            None => return Err(anyhow!("no saved library session")),
        };

        let session = Session::new(session_config, Some(cache.clone()));
        tokio::time::timeout(Duration::from_secs(30), session.connect(credentials, false))
            .await
            .map_err(|_| anyhow!("library session timed out"))??;
        Self::seal_session(&session);
        *self.user_id.write().await = session.username();
        *self.session.write().await = Some(session.clone());
        tracing::info!("library session ready as {}", session.username());
        Ok(())
    }

    fn eval(&self, script: String) {
        if let Ok(player) = self.player.lock()
            && let Some(player) = player.as_ref() {
                player.eval(script);
            }
    }

    fn call(&self, method: &str, args: &[Value]) {
        let args = args.iter().map(Value::to_string).collect::<Vec<_>>().join(",");
        let sep = if args.is_empty() { "" } else { "," };
        self.eval(format!("rustify.call({}{sep}{args})", json!(method)));
    }

    async fn listen(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<String>) {
        while let Some(raw) = rx.recv().await {
            let Ok(msg) = serde_json::from_str::<Value>(&raw) else { continue };
            match msg.get("type").and_then(Value::as_str).unwrap_or_default() {
                "token" => {
                    use std::sync::atomic::Ordering;
                    if self.token_pending.swap(true, Ordering::SeqCst) {
                        continue;
                    }
                    let engine = Arc::clone(&self);
                    tokio::spawn(async move {
                        let mut wait = 2u64;
                        for attempt in 0..60u32 {
                            match engine.auth.access_token().await {
                                Ok(token) => {
                                    engine.eval(format!("rustify.token({})", json!(token)));
                                    break;
                                }
                                Err(e) if attempt == 0 => tracing::warn!("player token: {e:#}, waiting for login"),
                                Err(_) => {}
                            }
                            tokio::time::sleep(Duration::from_secs(wait)).await;
                            wait = (wait * 2).min(30);
                        }
                        engine.token_pending.store(false, Ordering::SeqCst);
                    });
                }
                "ready" => {
                    let device = msg.get("device_id").and_then(Value::as_str).unwrap_or_default().to_owned();
                    tracing::info!("web player ready as device {device}");
                    *self.device.write().await = Some(device);
                    *self.status.write().await = "ready";
                    *self.error.write().await = None;
                    self.update(|s| s.ready = true).await;
                    self.push_audio();
                    let volume = self.state.read().await.volume;
                    let gain = self.audio.lock().map(|a| a.gain(volume)).unwrap_or(volume as f32 / 100.0);
                    self.call("setVolume", &[json!(gain)]);
                    if self.state.read().await.track.is_none() {
                        tokio::spawn(Arc::clone(&self).stage_last());
                    }
                }
                "not_ready" => {
                    *self.device.write().await = None;
                    *self.status.write().await = "connecting";
                    self.update(|s| s.ready = false).await;
                }
                "state" => {
                    self.settled();
                    let state = msg.get("state").cloned().unwrap_or(Value::Null);
                    self.apply(&state).await;
                }
                "error" => {
                    let kind = msg.get("kind").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let message = msg.get("message").and_then(Value::as_str).unwrap_or_default().to_owned();
                    tracing::warn!("web player {kind}: {message}");
                    match kind.as_str() {
                        "initialization_error" => {
                            self.set_error(Some("This PC can't play protected audio (WebView2 is missing or outdated)".into())).await
                        }
                        "account_error" => self.set_error(Some("Spotify Premium is required for playback".into())).await,
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    async fn stage_last(self: Arc<Self>) {
        if let Ok(current) = self.web.get::<Value>("/me/player").await
            && current.get("is_playing").and_then(Value::as_bool).unwrap_or(false) {
                return;
            }
        let remote = match self.web.get::<Value>("/me/player/recently-played?limit=1").await {
            Ok(recent) => recent.pointer("/items/0").and_then(|item| {
                let track = item.get("track").filter(|t| !t.is_null())?;
                let context = item.pointer("/context/uri").and_then(Value::as_str).map(str::to_owned);
                Some((context, now_playing(track)))
            }),
            Err(_) => None,
        };
        let Some((context, playing)) = remote.or_else(read_last) else { return };
        if playing.uri.is_empty() {
            return;
        }
        if let Ok(mut staged) = self.staged.lock() {
            *staged = Some((context.clone(), playing.uri.clone()));
        }
        self.update(|s| {
            if s.track.is_none() {
                s.track = Some(playing);
                s.context = context;
                s.is_playing = false;
                s.position_ms = 0;
            }
        })
        .await;
    }

    async fn remember_last(&self, context: Option<String>, playing: &NowPlaying) {
        let changed = self.state.read().await.track.as_ref().is_none_or(|t| t.uri != playing.uri);
        if !changed || playing.duration_ms < 15_000 {
            return;
        }
        if let Ok(raw) = serde_json::to_vec(&json!({ "context": context, "track": playing })) {
            let _ = std::fs::write(Config::data_dir().join("last.json"), raw);
        }
        let mut history = read_history();
        history.retain(|t| t.uri != playing.uri);
        history.insert(0, playing.clone());
        history.truncate(50);
        if let Ok(raw) = serde_json::to_vec(&history) {
            let _ = std::fs::write(Config::data_dir().join("history.json"), raw);
        }
    }

    pub async fn set_audio(&self, settings: AudioSettings) {
        let settings = settings.sanitized();
        if let Ok(mut audio) = self.audio.lock() {
            *audio = settings.clone();
        }
        self.eval(format!("window.rustifyAudio && rustifyAudio.apply({})", json!(settings)));
        let volume = self.state.read().await.volume;
        let gain = settings.gain(volume);
        self.call("setVolume", &[json!(gain)]);
    }

    fn push_audio(&self) {
        let settings = self.audio.lock().map(|a| a.clone()).unwrap_or_default();
        self.eval(format!("window.rustifyAudio && rustifyAudio.apply({})", json!(settings)));
    }

    pub fn staged(&self) -> Option<(Option<String>, String)> {
        self.staged.lock().ok().and_then(|s| s.clone())
    }

    fn take_staged(&self) -> Option<(Option<String>, String)> {
        self.staged.lock().ok().and_then(|mut s| s.take())
    }

    async fn apply(&self, state: &Value) {
        if state.is_null() {
            self.update(|s| s.is_playing = false).await;
            return;
        }
        let track = state.get("track").filter(|t| !t.is_null()).map(now_playing);
        let paused = state.get("paused").and_then(Value::as_bool).unwrap_or(true);
        let position = state.get("position").and_then(Value::as_u64).unwrap_or(0) as u32;
        let shuffle = state.get("shuffle").and_then(Value::as_bool).unwrap_or(false);
        let repeat = match state.get("repeat").and_then(Value::as_u64).unwrap_or(0) {
            1 => "context",
            2 => "track",
            _ => "off",
        };
        let context = state.get("context").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned);
        if let Some(playing) = &track {
            self.take_staged();
            self.remember_last(context.clone(), playing).await;
        }
        if !paused {
            self.set_remote(None).await;
        }
        self.update(|s| {
            if track.is_some() {
                s.track = track;
                s.context = context;
            }
            s.is_playing = !paused;
            s.position_ms = position;
            s.shuffle = shuffle;
            s.repeat = repeat.into();
        })
        .await;
    }

    async fn update(&self, f: impl FnOnce(&mut PlayerState)) {
        let snapshot = {
            let mut state = self.state.write().await;
            f(&mut state);
            state.clone()
        };
        if self.remote.read().await.is_none() {
            let _ = self.events.send(snapshot);
        }
    }

    fn begin(&self, command: &'static str) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some((command, Instant::now()));
        }
    }

    fn settled(&self) {
        if let Some((command, at)) = self.pending.lock().ok().and_then(|mut p| p.take()) {
            tracing::info!("latency: {command} -> state in {} ms", at.elapsed().as_millis());
        }
    }

    async fn require_device(&self) -> anyhow::Result<String> {
        self.device().await.ok_or_else(|| anyhow!("player not ready"))
    }

    async fn user_id(&self) -> anyhow::Result<String> {
        let cached = self.user_id.read().await.clone();
        if !cached.is_empty() {
            return Ok(cached);
        }
        let id = match self.web.get::<Value>("/me").await {
            Ok(me) => me.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
            Err(e) => match self.session().await {
                Some(session) => session.username(),
                None => return Err(e),
            },
        };
        if id.is_empty() {
            return Err(anyhow!("could not resolve your Spotify account"));
        }
        *self.user_id.write().await = id.clone();
        Ok(id)
    }

    async fn radio(&self, track_uri: &str) -> Vec<String> {
        let Some(session) = self.session().await else { return Vec::new() };
        let Ok(uri) = SpotifyUri::from_uri(track_uri) else { return Vec::new() };
        let Ok(bytes) = session.spclient().get_radio_for_track(&uri).await else { return Vec::new() };
        serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.get("mediaItems").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|item| item.get("uri").and_then(Value::as_str).map(str::to_owned))
            .filter(|uri| uri != track_uri)
            .take(50)
            .collect()
    }

    pub async fn play(&self, context_uri: Option<String>, uris: Vec<String>, offset: Option<String>, radio: bool) -> anyhow::Result<()> {
        let device = self.require_device().await?;
        let body = match context_uri {
            Some(track) if radio => {
                let mut list = vec![track.clone()];
                list.extend(self.radio(&track).await);
                json!({ "uris": list })
            }
            Some(uri) => {
                let context = if uri == "collection" {
                    format!("spotify:user:{}:collection", self.user_id().await?)
                } else {
                    uri
                };
                match offset {
                    Some(track) => json!({ "context_uri": context, "offset": { "uri": track } }),
                    None => json!({ "context_uri": context }),
                }
            }
            None if !uris.is_empty() => match offset {
                Some(track) => json!({ "uris": uris, "offset": { "uri": track } }),
                None => json!({ "uris": uris }),
            },
            None => {
                if let Some(result) = self.remote_command("PUT", "/me/player/play", None).await {
                    return result;
                }
                self.begin("resume");
                self.call("resume", &[]);
                return Ok(());
            }
        };
        if let Some(result) = self.remote_command("PUT", "/me/player/play", Some(body.clone())).await {
            return result;
        }
        self.begin("load");
        self.web.put(&format!("/me/player/play?device_id={device}"), Some(body)).await
    }

    pub async fn take_staged_for_toggle(&self) -> Option<(Option<String>, String)> {
        if self.remote().await.is_some() {
            return None;
        }
        self.take_staged()
    }

    pub async fn toggle(&self) -> anyhow::Result<()> {
        if self.remote().await.is_some() {
            let playing = self.current().await.is_playing;
            let path = if playing { "/me/player/pause" } else { "/me/player/play" };
            if let Some(result) = self.remote_command("PUT", path, None).await {
                return result;
            }
        }
        if let Some((context, uri)) = self.take_staged() {
            let attempt = match context.clone() {
                Some(context) => self.play(Some(context), Vec::new(), Some(uri.clone()), false).await,
                None => self.play(None, vec![uri.clone()], None, false).await,
            };
            if attempt.is_err() && context.is_some() {
                return self.play(None, vec![uri], None, false).await;
            }
            return attempt;
        }
        self.begin("toggle");
        self.call("togglePlay", &[]);
        Ok(())
    }

    pub async fn pause(&self) -> anyhow::Result<()> {
        if let Some(result) = self.remote_command("PUT", "/me/player/pause", None).await {
            return result;
        }
        self.begin("pause");
        self.call("pause", &[]);
        Ok(())
    }

    pub async fn next(&self) -> anyhow::Result<()> {
        if let Some(result) = self.remote_command("POST", "/me/player/next", None).await {
            return result;
        }
        self.begin("next");
        self.call("nextTrack", &[]);
        Ok(())
    }

    pub async fn prev(&self) -> anyhow::Result<()> {
        if let Some(result) = self.remote_command("POST", "/me/player/previous", None).await {
            return result;
        }
        self.begin("prev");
        self.call("previousTrack", &[]);
        Ok(())
    }

    pub async fn seek(&self, position_ms: u32) -> anyhow::Result<()> {
        if let Some(result) = self.remote_command("PUT", &format!("/me/player/seek?position_ms={position_ms}"), None).await {
            return result;
        }
        self.begin("seek");
        self.call("seek", &[json!(position_ms)]);
        Ok(())
    }

    pub async fn volume(&self, percent: u8, commit: bool) -> anyhow::Result<()> {
        let percent = percent.min(100);
        if self.remote().await.is_some() {
            if !commit {
                return Ok(());
            }
            if let Some(result) = self.remote_command("PUT", &format!("/me/player/volume?volume_percent={percent}"), None).await {
                return result;
            }
        }
        let gain = self.audio.lock().map(|a| a.gain(percent)).unwrap_or(percent as f32 / 100.0);
        self.call("setVolume", &[json!(gain)]);
        if commit {
            self.update(|s| s.volume = percent).await;
        } else {
            self.state.write().await.volume = percent;
        }
        Ok(())
    }

    pub async fn shuffle(&self, on: bool) -> anyhow::Result<()> {
        if let Some(result) = self.remote_command("PUT", &format!("/me/player/shuffle?state={on}"), None).await {
            return result;
        }
        let device = self.require_device().await?;
        self.web.put(&format!("/me/player/shuffle?state={on}&device_id={device}"), None).await?;
        self.update(|s| s.shuffle = on).await;
        Ok(())
    }

    pub async fn repeat(&self, mode: &str) -> anyhow::Result<()> {
        let mode = match mode {
            "context" | "track" => mode,
            _ => "off",
        };
        if let Some(result) = self.remote_command("PUT", &format!("/me/player/repeat?state={mode}"), None).await {
            return result;
        }
        let device = self.require_device().await?;
        self.web.put(&format!("/me/player/repeat?state={mode}&device_id={device}"), None).await?;
        self.update(|s| s.repeat = mode.into()).await;
        Ok(())
    }
}

pub fn read_history() -> Vec<NowPlaying> {
    std::fs::read(Config::data_dir().join("history.json"))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default()
}

fn read_last() -> Option<(Option<String>, NowPlaying)> {
    let raw = std::fs::read(Config::data_dir().join("last.json")).ok()?;
    let value: Value = serde_json::from_slice(&raw).ok()?;
    let context = value.get("context").and_then(Value::as_str).map(str::to_owned);
    let track = serde_json::from_value(value.get("track")?.clone()).ok()?;
    Some((context, track))
}

fn now_playing(track: &Value) -> NowPlaying {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    let id_of = |uri: &str| uri.rsplit(':').next().unwrap_or_default().to_owned();

    let album = track.get("album").cloned().unwrap_or_default();
    let mut images: Vec<(u64, String)> = album
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|img| {
            let url = img.get("url").and_then(Value::as_str)?.to_owned();
            let size = img.get("width").or_else(|| img.get("height")).and_then(Value::as_u64).unwrap_or(0);
            Some((size, url))
        })
        .collect();
    images.sort_by_key(|(size, _)| *size);
    let small = images.iter().find(|(size, _)| *size >= 64).or(images.first()).map(|(_, u)| u.clone());
    let large = images.last().map(|(_, u)| u.clone());

    NowPlaying {
        id: {
            let id = text(track, "id");
            if id.is_empty() { id_of(&text(track, "uri")) } else { id }
        },
        uri: text(track, "uri"),
        name: text(track, "name"),
        artists: track
            .get("artists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|a| ArtistRef { id: id_of(&text(a, "uri")), name: text(a, "name") })
            .collect(),
        album: text(&album, "name"),
        image_small: small,
        image_large: large,
        duration_ms: track.get("duration").or_else(|| track.get("duration_ms")).and_then(Value::as_u64).unwrap_or(0) as u32,
        explicit: track.get("explicit").and_then(Value::as_bool).unwrap_or(false),
    }
}
