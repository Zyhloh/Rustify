use super::manifest::{self, Manifest};
use rquickjs::{Context, Ctx, Exception, Function, Runtime};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

const PRELUDE: &str = include_str!("prelude.js");
pub const MEMORY_LIMIT: usize = 32 * 1024 * 1024;
const STACK_LIMIT: usize = 1024 * 1024;
pub const CALL_BUDGET: Duration = Duration::from_millis(250);
const LOAD_BUDGET: Duration = Duration::from_secs(2);
const MAX_STRIKES: u32 = 3;
const STORAGE_LIMIT: usize = 1024 * 1024;
const MAX_TIMERS: usize = 100;
const UI_OPS_PER_SECOND: u32 = 120;
const SPOTIFY_PER_MINUTE: usize = 30;
const NET_PER_MINUTE: usize = 120;
const NET_BODY_LIMIT: usize = 1024 * 1024;
const BLOCKED_HEADERS: [&str; 8] = ["host", "cookie", "proxy-authorization", "connection", "content-length", "transfer-encoding", "origin", "referer"];
pub const HIDEABLE: [&str; 8] = [
    "friends-button", "news-button", "now-playing-button", "lyrics-button", "queue-button", "devices-button", "mini-player-button", "fullscreen-button",
];

pub struct Spec {
    pub manifest: Manifest,
    pub script: String,
    pub storage_dir: PathBuf,
    pub state: Option<String>,
}

pub enum Cmd {
    Load(Box<Spec>),
    Unload(String),
    Emit(String, String),
    EmitTo(String, String, String),
    Action { plugin: String, key: String, json: String },
    Resolve { plugin: String, id: u32, ok: bool, json: String },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum UiOp {
    Log { plugin: String, level: String, text: String },
    Notify(String),
    Player { action: String, value: f64 },
    Control { plugin: String, place: String, id: String, label: String, icon: String, active: bool, remove: bool },
    Menu { plugin: String, id: String, label: String, types: Vec<String>, remove: bool },
    Page { plugin: String, id: String, title: String, icon: String, handle: String, remove: bool },
    Section { plugin: String, place: String, id: String, title: String, handle: String, remove: bool },
    SectionRender { plugin: String, id: String, nodes: Vec<Value> },
    Hide { plugin: String, element: String, hidden: bool },
    Clipboard { plugin: String, text: String },
    Audio { plugin: String, payload: Value },
    Render { plugin: String, id: String, nodes: Vec<Value> },
    Open { plugin: String, id: String },
    Theme { plugin: String, colors: BTreeMap<String, String> },
    ThemeReset(String),
    Navigate(String),
    Started(String),
    Failed { plugin: String, error: String },
    Stopped(String),
}

pub type Sink = Arc<dyn Fn(UiOp) + Send + Sync>;
#[derive(Debug, Clone)]
pub enum Request {
    Spotify(String),
    Net { url: String, method: String, headers: Vec<(String, String)>, body: Option<String>, hosts: Vec<String> },
}

impl std::fmt::Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Request::Spotify(path) => f.write_str(path),
            Request::Net { method, url, .. } => write!(f, "{method} {url}"),
        }
    }
}

pub type Fetch = Arc<dyn Fn(Request, Box<dyn FnOnce(Result<String, String>) + Send>) + Send + Sync>;

struct Shared {
    id: String,
    perms: HashSet<String>,
    storage: BTreeMap<String, String>,
    storage_path: PathBuf,
    timers: BTreeMap<u32, (Instant, Option<Duration>)>,
    next_request: u32,
    spotify_calls: VecDeque<Instant>,
    net_calls: VecDeque<Instant>,
    hosts: Vec<String>,
    last_notify: Option<Instant>,
    last_clipboard: Option<Instant>,
    ui_window: (Instant, u32),
    sink: Sink,
    fetch: Fetch,
    tx: mpsc::Sender<Cmd>,
}

struct Instance {
    shared: Rc<RefCell<Shared>>,
    deadline: Rc<Cell<Option<Instant>>>,
    strikes: u32,
    ctx: Context,
    rt: Runtime,
}

fn fail(message: impl Into<String>) -> Result<String, String> {
    Err(message.into())
}

fn arg<'a>(payload: &'a Value, key: &str) -> &'a Value {
    payload.get(key).unwrap_or(&Value::Null)
}

fn string(payload: &Value, key: &str, max: usize) -> String {
    arg(payload, key).as_str().unwrap_or_default().chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect()
}

pub fn valid_icon(path: &str) -> bool {
    !path.is_empty() && path.len() <= 2000 && path.bytes().all(|b| b"MmLlHhVvCcSsQqTtAaZz0123456789 .,-".contains(&b))
}

pub fn valid_uri(uri: &str) -> bool {
    let mut parts = uri.splitn(3, ':');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some("spotify"), Some("track" | "album" | "artist" | "playlist" | "show" | "episode"), Some(id))
            if (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
    )
}

pub fn valid_api_path(path: &str) -> bool {
    path.starts_with("/v1/")
        && path.len() <= 512
        && !path.contains("..")
        && !path.contains("//")
        && path.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-.?=&,%:+".contains(&b))
}

fn valid_local_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Shared {
    fn ui(&mut self, op: UiOp) -> Result<(), String> {
        let now = Instant::now();
        if now.duration_since(self.ui_window.0) >= Duration::from_secs(1) {
            self.ui_window = (now, 0);
        }
        self.ui_window.1 += 1;
        if self.ui_window.1 > UI_OPS_PER_SECOND {
            return Err("too many UI updates, slow down".into());
        }
        (self.sink)(op);
        Ok(())
    }

    fn need(&self, permission: &str) -> Result<(), String> {
        if self.perms.contains(permission) {
            Ok(())
        } else {
            Err(format!("this plugin does not have the \"{permission}\" permission"))
        }
    }

    fn storage_size(&self) -> usize {
        self.storage.iter().map(|(k, v)| k.len() + v.len()).sum()
    }

    fn save_storage(&self) {
        if let Some(dir) = self.storage_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(raw) = serde_json::to_vec(&self.storage) {
            let _ = std::fs::write(&self.storage_path, raw);
        }
    }

    fn op(&mut self, op: &str, payload: &Value) -> Result<String, String> {
        let id = self.id.clone();
        match op {
            "info" => Ok(json!({ "version": env!("CARGO_PKG_VERSION"), "plugin": id }).to_string()),
            "log" => {
                let level = match arg(payload, "level").as_str() {
                    Some("warn") => "warn",
                    Some("error") => "error",
                    _ => "info",
                };
                self.ui(UiOp::Log { plugin: id, level: level.into(), text: string(payload, "text", 2000) })?;
                Ok(String::new())
            }
            "notify" => {
                let now = Instant::now();
                if self.last_notify.is_some_and(|t| now.duration_since(t) < Duration::from_secs(2)) {
                    return Ok(String::new());
                }
                self.last_notify = Some(now);
                let text = string(payload, "text", 200);
                if !text.trim().is_empty() {
                    self.ui(UiOp::Notify(text))?;
                }
                Ok(String::new())
            }
            "player" => {
                self.need("player")?;
                let action = string(payload, "action", 16);
                if !matches!(action.as_str(), "play" | "pause" | "toggle" | "next" | "prev" | "seek" | "volume" | "shuffle" | "repeat" | "like") {
                    return fail("unknown player action");
                }
                let value = arg(payload, "value").as_f64().filter(|v| v.is_finite()).unwrap_or(0.0);
                self.ui(UiOp::Player { action, value })?;
                Ok(String::new())
            }
            "storage.get" => Ok(self.storage.get(&string(payload, "key", 128)).map(|v| json!(v).to_string()).unwrap_or_else(|| "null".into())),
            "storage.keys" => Ok(json!(self.storage.keys().collect::<Vec<_>>()).to_string()),
            "storage.set" => {
                let key = string(payload, "key", 128);
                let value = arg(payload, "value").as_str().unwrap_or_default().to_owned();
                let before = self.storage.get(&key).map(|v| key.len() + v.len()).unwrap_or(0);
                if self.storage_size() - before + key.len() + value.len() > STORAGE_LIMIT {
                    return fail("plugin storage is full (1 MB)");
                }
                self.storage.insert(key, value);
                self.save_storage();
                Ok(String::new())
            }
            "storage.remove" => {
                self.storage.remove(&string(payload, "key", 128));
                self.save_storage();
                Ok(String::new())
            }
            "storage.clear" => {
                self.storage.clear();
                self.save_storage();
                Ok(String::new())
            }
            "timer.set" => {
                if self.timers.len() >= MAX_TIMERS {
                    return fail("too many timers");
                }
                let tid = arg(payload, "id").as_u64().unwrap_or(0) as u32;
                let repeat = arg(payload, "repeat").as_bool().unwrap_or(false);
                let ms = arg(payload, "ms").as_f64().unwrap_or(0.0).clamp(0.0, 86_400_000.0) as u64;
                let ms = if repeat { ms.max(100) } else { ms.max(4) };
                let every = Duration::from_millis(ms);
                self.timers.insert(tid, (Instant::now() + every, repeat.then_some(every)));
                Ok(String::new())
            }
            "timer.clear" => {
                self.timers.remove(&(arg(payload, "id").as_u64().unwrap_or(0) as u32));
                Ok(String::new())
            }
            "ui.topbar" | "ui.playbar" | "ui.trackrow" => {
                let control = string(payload, "id", 40);
                let remove = arg(payload, "remove").as_bool().unwrap_or(false);
                let icon = string(payload, "icon", 2000);
                if !remove && !valid_icon(&icon) {
                    return fail("icon must be an SVG path in a 24x24 box");
                }
                let place = match op {
                    "ui.topbar" => "topbar",
                    "ui.playbar" => "playbar",
                    _ => "trackrow",
                };
                self.ui(UiOp::Control {
                    plugin: id,
                    place: place.into(),
                    id: control,
                    label: string(payload, "label", 40),
                    icon,
                    active: arg(payload, "active").as_bool().unwrap_or(false),
                    remove,
                })?;
                Ok(String::new())
            }
            "ui.menu" => {
                let types = arg(payload, "types")
                    .as_array()
                    .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_owned)).take(6).collect())
                    .unwrap_or_default();
                self.ui(UiOp::Menu {
                    plugin: id,
                    id: string(payload, "id", 40),
                    label: string(payload, "label", 40),
                    types,
                    remove: arg(payload, "remove").as_bool().unwrap_or(false),
                })?;
                Ok(String::new())
            }
            "ui.page" => {
                let page = string(payload, "id", 40);
                if !valid_local_id(&page) {
                    return fail("page id must be 1-40 characters of a-z, 0-9, - and _");
                }
                let remove = arg(payload, "remove").as_bool().unwrap_or(false);
                let icon = string(payload, "icon", 2000);
                if !remove && !valid_icon(&icon) {
                    return fail("icon must be an SVG path in a 24x24 box");
                }
                self.ui(UiOp::Page {
                    plugin: id,
                    id: page,
                    title: string(payload, "title", 60),
                    icon,
                    handle: string(payload, "handle", 20),
                    remove,
                })?;
                Ok(String::new())
            }
            "ui.render" => {
                let nodes = arg(payload, "nodes").as_array().cloned().unwrap_or_default();
                if nodes.len() > 300 {
                    return fail("a page can show at most 300 items");
                }
                self.ui(UiOp::Render { plugin: id, id: string(payload, "id", 40), nodes })?;
                Ok(String::new())
            }
            "ui.section" => {
                let section = string(payload, "id", 40);
                if !valid_local_id(&section) {
                    return fail("section id must be 1-40 characters of a-z, 0-9, - and _");
                }
                let place = string(payload, "place", 20);
                if !matches!(place.as_str(), "now-playing" | "home" | "search" | "queue" | "dialog") {
                    return fail("sections can go in \"now-playing\", \"home\", \"search\", \"queue\" or \"dialog\"");
                }
                self.ui(UiOp::Section {
                    plugin: id,
                    place,
                    id: section,
                    title: string(payload, "title", 60),
                    handle: string(payload, "handle", 20),
                    remove: arg(payload, "remove").as_bool().unwrap_or(false),
                })?;
                Ok(String::new())
            }
            "ui.section.render" => {
                let nodes = arg(payload, "nodes").as_array().cloned().unwrap_or_default();
                if nodes.len() > 100 {
                    return fail("a section can show at most 100 items");
                }
                self.ui(UiOp::SectionRender { plugin: id, id: string(payload, "id", 40), nodes })?;
                Ok(String::new())
            }
            "ui.hide" => {
                let element = string(payload, "element", 40);
                if !HIDEABLE.contains(&element.as_str()) {
                    return fail(format!("can't hide \"{element}\". Use one of: {}", HIDEABLE.join(", ")));
                }
                self.ui(UiOp::Hide { plugin: id, element, hidden: arg(payload, "hidden").as_bool().unwrap_or(true) })?;
                Ok(String::new())
            }
            "audio" => {
                self.need("audio")?;
                let action = string(payload, "action", 10);
                let mut out = json!({ "action": action, "value": arg(payload, "value").as_f64().filter(|v| v.is_finite()).unwrap_or(0.0) });
                match action.as_str() {
                    "load" => {
                        let track = arg(payload, "track");
                        let url = string(track, "url", 2000);
                        if !manifest::host_allowed(&self.hosts, &url) {
                            return fail(format!("audio can only come from {} over https", self.hosts.join(", ")));
                        }
                        let image = string(track, "image", 2000);
                        let image = if manifest::host_allowed(&self.hosts, &image) || crate::images::trusted(&image) { image } else { String::new() };
                        out["track"] = json!({
                            "url": url,
                            "title": string(track, "title", 200),
                            "artist": string(track, "artist", 200),
                            "album": string(track, "album", 200),
                            "image": image,
                            "id": string(track, "id", 100),
                            "start_ms": arg(track, "start").as_f64().filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0).min(86_400_000.0) as u64,
                        });
                    }
                    "play" | "pause" | "toggle" | "seek" | "stop" => {}
                    _ => return fail("audio action must be load, play, pause, toggle, seek or stop"),
                }
                self.ui(UiOp::Audio { plugin: id, payload: out })?;
                Ok(String::new())
            }
            "clipboard" => {
                let now = Instant::now();
                if self.last_clipboard.is_some_and(|t| now.duration_since(t) < Duration::from_secs(1)) {
                    return fail("copy at most once a second");
                }
                self.last_clipboard = Some(now);
                self.ui(UiOp::Clipboard { plugin: id, text: string(payload, "text", 5000) })?;
                Ok(String::new())
            }
            "ui.open" => {
                self.ui(UiOp::Open { plugin: id, id: string(payload, "id", 40) })?;
                Ok(String::new())
            }
            "theme.set" => {
                self.need("theme")?;
                let colors: BTreeMap<String, String> = payload
                    .as_object()
                    .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned()))).collect())
                    .unwrap_or_default();
                manifest::check_theme(&colors)?;
                self.ui(UiOp::Theme { plugin: id, colors })?;
                Ok(String::new())
            }
            "theme.reset" => {
                self.need("theme")?;
                self.ui(UiOp::ThemeReset(id))?;
                Ok(String::new())
            }
            "navigate" => {
                let uri = string(payload, "uri", 200);
                if !valid_uri(&uri) {
                    return fail("navigate needs a spotify:track/album/artist/playlist/show uri");
                }
                self.ui(UiOp::Navigate(uri))?;
                Ok(String::new())
            }
            _ => fail(format!("unknown operation \"{op}\"")),
        }
    }

    fn op_async(&mut self, op: &str, payload: &Value) -> Result<u32, String> {
        match op {
            "spotify.get" => {
                self.need("spotify")?;
                let path = string(payload, "path", 512);
                if !valid_api_path(&path) {
                    return Err("path must look like /v1/... on api.spotify.com".into());
                }
                let now = Instant::now();
                while self.spotify_calls.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60)) {
                    self.spotify_calls.pop_front();
                }
                if self.spotify_calls.len() >= SPOTIFY_PER_MINUTE {
                    return Err("Spotify request limit reached (30 per minute)".into());
                }
                self.spotify_calls.push_back(now);
                Ok(self.request(Request::Spotify(path)))
            }
            "net.fetch" => {
                self.need("network")?;
                let url = string(payload, "url", 2000);
                if !manifest::host_allowed(&self.hosts, &url) {
                    return Err(format!("this plugin can only connect to {} over https", self.hosts.join(", ")));
                }
                let method = string(payload, "method", 10).to_ascii_uppercase();
                let method = if method.is_empty() { "GET".to_owned() } else { method };
                if !matches!(method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD") {
                    return Err("method must be GET, POST, PUT, PATCH, DELETE or HEAD".into());
                }
                let headers: Vec<(String, String)> = arg(payload, "headers")
                    .as_object()
                    .map(|o| {
                        o.iter()
                            .take(30)
                            .filter_map(|(k, v)| v.as_str().map(|v| (k.to_ascii_lowercase(), v.chars().take(1000).collect::<String>())))
                            .filter(|(k, v)| {
                                !BLOCKED_HEADERS.contains(&k.as_str())
                                    && !k.starts_with("sec-")
                                    && k.len() <= 64
                                    && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                                    && !v.contains(['\r', '\n'])
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let body = arg(payload, "body").as_str().map(str::to_owned);
                if body.as_ref().is_some_and(|b| b.len() > NET_BODY_LIMIT) {
                    return Err("request body is larger than 1 MB".into());
                }
                let now = Instant::now();
                while self.net_calls.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60)) {
                    self.net_calls.pop_front();
                }
                if self.net_calls.len() >= NET_PER_MINUTE {
                    return Err("request limit reached (120 per minute)".into());
                }
                self.net_calls.push_back(now);
                Ok(self.request(Request::Net { url, method, headers, body, hosts: self.hosts.clone() }))
            }
            _ => Err(format!("unknown operation \"{op}\"")),
        }
    }

    fn request(&mut self, request: Request) -> u32 {
        self.next_request += 1;
        let id = self.next_request;
        let (tx, plugin) = (self.tx.clone(), self.id.clone());
        (self.fetch)(
            request,
            Box::new(move |result| {
                let (ok, json) = match result {
                    Ok(json) => (true, json),
                    Err(e) => (false, e),
                };
                let _ = tx.send(Cmd::Resolve { plugin, id, ok, json });
            }),
        );
        id
    }
}

fn exception_text(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if let rquickjs::Error::Exception = error {
        let caught = ctx.catch();
        if let Some(ex) = caught.as_exception() {
            let message = ex.message().unwrap_or_default();
            return match ex.stack() {
                Some(stack) if !stack.is_empty() => format!("{message}\n{stack}"),
                _ => message,
            };
        }
        if let Some(s) = caught.as_string().and_then(|s| s.to_string().ok()) {
            return s;
        }
        return "uncaught exception".into();
    }
    error.to_string()
}

impl std::fmt::Debug for Instance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Instance")
    }
}

impl Instance {
    fn start(spec: Spec, sink: Sink, fetch: Fetch, tx: mpsc::Sender<Cmd>) -> Result<Self, String> {
        let m = spec.manifest;
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        rt.set_memory_limit(MEMORY_LIMIT);
        rt.set_max_stack_size(STACK_LIMIT);
        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let watch = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || watch.get().is_some_and(|d| Instant::now() > d))));
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        let storage_path = spec.storage_dir.join(format!("{}.json", m.id));
        let storage = std::fs::read(&storage_path).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default();
        let shared = Rc::new(RefCell::new(Shared {
            id: m.id.clone(),
            perms: m.permissions.iter().cloned().collect(),
            storage,
            storage_path,
            timers: BTreeMap::new(),
            next_request: 0,
            spotify_calls: VecDeque::new(),
            net_calls: VecDeque::new(),
            hosts: m.network.clone(),
            last_notify: None,
            last_clipboard: None,
            ui_window: (Instant::now(), 0),
            sink,
            fetch,
            tx,
        }));
        let instance = Self { shared: shared.clone(), deadline, strikes: 0, ctx, rt };
        let (sync, asynchronous) = (shared.clone(), shared);
        let theme = m.theme.clone();
        let started = instance.run(LOAD_BUDGET, |ctx| {
            let globals = ctx.globals();
            globals.set(
                "__host",
                Function::new(ctx.clone(), move |ctx: Ctx<'_>, op: String, payload: String| -> rquickjs::Result<String> {
                    let payload: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
                    let result = sync.borrow_mut().op(&op, &payload);
                    result.map_err(|e| Exception::throw_message(&ctx, &e))
                })?,
            )?;
            globals.set(
                "__hostAsync",
                Function::new(ctx.clone(), move |ctx: Ctx<'_>, op: String, payload: String| -> rquickjs::Result<u32> {
                    let payload: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
                    let result = asynchronous.borrow_mut().op_async(&op, &payload);
                    result.map_err(|e| Exception::throw_message(&ctx, &e))
                })?,
            )?;
            ctx.eval::<(), _>(PRELUDE)?;
            if let Some(state) = &spec.state {
                let emit: Function = ctx.globals().get("__emit")?;
                emit.call::<_, ()>(("state", state.as_str()))?;
            }
            ctx.eval::<(), _>(spec.script.as_str())
        });
        started?;
        if !theme.is_empty() {
            let _ = instance.shared.borrow_mut().ui(UiOp::Theme { plugin: m.id.clone(), colors: theme });
        }
        Ok(instance)
    }

    fn run(&self, budget: Duration, f: impl for<'js> FnOnce(&Ctx<'js>) -> rquickjs::Result<()>) -> Result<(), String> {
        self.deadline.set(Some(Instant::now() + budget));
        let result = self.ctx.with(|ctx| f(&ctx).map_err(|e| exception_text(&ctx, e)));
        let mut jobs = 0;
        while result.is_ok() && self.rt.is_job_pending() && jobs < 10_000 {
            if self.rt.execute_pending_job().is_err() {
                break;
            }
            jobs += 1;
        }
        self.deadline.set(None);
        result
    }

    fn call(&mut self, name: &'static str, a: String, b: Option<String>) -> Result<(), String> {
        let result = self.run(CALL_BUDGET, |ctx| {
            let f: Function = ctx.globals().get(name)?;
            match &b {
                Some(b) => f.call::<_, ()>((a.as_str(), b.as_str())),
                None => f.call::<_, ()>((a.as_str(),)),
            }
        });
        if let Err(e) = &result
            && e.contains("interrupted") {
                self.strikes += 1;
            }
        result
    }

    fn call_resolve(&mut self, id: u32, ok: bool, json: String) -> Result<(), String> {
        self.run(CALL_BUDGET, |ctx| {
            let f: Function = ctx.globals().get("__resolve")?;
            f.call::<_, ()>((id, ok, json.as_str()))
        })
    }

    fn fire(&mut self, id: u32) -> Result<(), String> {
        let result = self.run(CALL_BUDGET, |ctx| {
            let f: Function = ctx.globals().get("__fire")?;
            f.call::<_, ()>((id,))
        });
        if let Err(e) = &result
            && e.contains("interrupted") {
                self.strikes += 1;
            }
        result
    }
}

pub fn spawn(sink: Sink, fetch: Fetch) -> mpsc::Sender<Cmd> {
    let (tx, rx) = mpsc::channel::<Cmd>();
    let own = tx.clone();
    std::thread::Builder::new()
        .name("rustify-plugins".into())
        .spawn(move || run(rx, own, sink, fetch))
        .expect("plugin thread");
    tx
}

fn report(sink: &Sink, plugin: &str, error: &str) {
    sink(UiOp::Log { plugin: plugin.into(), level: "error".into(), text: error.chars().take(2000).collect() });
}

fn run(rx: mpsc::Receiver<Cmd>, tx: mpsc::Sender<Cmd>, sink: Sink, fetch: Fetch) {
    let mut plugins: HashMap<String, Instance> = HashMap::new();
    loop {
        let next = plugins
            .values()
            .filter_map(|p| p.shared.borrow().timers.values().map(|(due, _)| *due).min())
            .min();
        let wait = next.map(|due| due.saturating_duration_since(Instant::now())).unwrap_or(Duration::from_secs(3600));
        match rx.recv_timeout(wait) {
            Ok(Cmd::Shutdown) => break,
            Ok(Cmd::Load(spec)) => {
                let id = spec.manifest.id.clone();
                plugins.remove(&id);
                match Instance::start(*spec, sink.clone(), fetch.clone(), tx.clone()) {
                    Ok(instance) => {
                        plugins.insert(id.clone(), instance);
                        sink(UiOp::Started(id));
                    }
                    Err(error) => {
                        report(&sink, &id, &error);
                        sink(UiOp::Failed { plugin: id, error: first_line(&error) });
                    }
                }
            }
            Ok(Cmd::Unload(id)) => {
                if plugins.remove(&id).is_some() {
                    sink(UiOp::Stopped(id));
                }
            }
            Ok(Cmd::Emit(kind, payload)) => {
                for (id, p) in plugins.iter_mut() {
                    if let Err(e) = p.call("__emit", kind.clone(), Some(payload.clone())) {
                        report(&sink, id, &e);
                    }
                }
            }
            Ok(Cmd::EmitTo(plugin, kind, payload)) => {
                if let Some(p) = plugins.get_mut(&plugin)
                    && let Err(e) = p.call("__emit", kind, Some(payload))
                {
                    report(&sink, &plugin, &e);
                }
            }
            Ok(Cmd::Action { plugin, key, json }) => {
                if let Some(p) = plugins.get_mut(&plugin)
                    && let Err(e) = p.call("__action", key, Some(json)) {
                        report(&sink, &plugin, &e);
                    }
            }
            Ok(Cmd::Resolve { plugin, id, ok, json }) => {
                if let Some(p) = plugins.get_mut(&plugin)
                    && let Err(e) = p.call_resolve(id, ok, json) {
                        report(&sink, &plugin, &e);
                    }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let now = Instant::now();
        for (id, p) in plugins.iter_mut() {
            let due: Vec<u32> = {
                let mut shared = p.shared.borrow_mut();
                let ids: Vec<u32> = shared.timers.iter().filter(|(_, (at, _))| *at <= now).map(|(k, _)| *k).collect();
                for k in &ids {
                    match shared.timers.get(k).and_then(|(_, every)| *every) {
                        Some(every) => {
                            shared.timers.insert(*k, (now + every, Some(every)));
                        }
                        None => {
                            shared.timers.remove(k);
                        }
                    }
                }
                ids
            };
            for timer in due {
                if let Err(e) = p.fire(timer) {
                    report(&sink, id, &e);
                }
            }
        }

        let broken: Vec<String> = plugins.iter().filter(|(_, p)| p.strikes >= MAX_STRIKES).map(|(id, _)| id.clone()).collect();
        for id in broken {
            plugins.remove(&id);
            sink(UiOp::Failed { plugin: id, error: "stopped because it kept freezing (ran longer than 250 ms)".into() });
        }
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn plugin(id: &str, perms: &[&str], script: &str) -> (Result<Instance, String>, Arc<Mutex<Vec<UiOp>>>) {
        let ops = Arc::new(Mutex::new(Vec::new()));
        let seen = ops.clone();
        let sink: Sink = Arc::new(move |op| seen.lock().unwrap().push(op));
        let fetch: Fetch = Arc::new(|_, done| done(Ok("{\"ok\":true}".into())));
        let (tx, _rx) = mpsc::channel();
        let manifest = Manifest {
            id: id.into(),
            name: id.into(),
            version: "1".into(),
            author: String::new(),
            description: String::new(),
            main: "index.js".into(),
            permissions: perms.iter().map(|p| p.to_string()).collect(),
            theme: BTreeMap::new(),
            ..Manifest::default()
        };
        let dir = std::env::temp_dir().join(format!("rustify-host-test-{}", std::process::id()));
        (Instance::start(Spec { manifest, script: script.into(), storage_dir: dir, state: None }, sink, fetch, tx), ops)
    }

    fn logs(ops: &Arc<Mutex<Vec<UiOp>>>) -> Vec<String> {
        ops.lock().unwrap().iter().filter_map(|o| if let UiOp::Log { text, .. } = o { Some(text.clone()) } else { None }).collect()
    }

    #[test]
    fn runs_plugin_and_logs() {
        let (p, ops) = plugin("hello", &[], "console.log('hi', Rustify.plugin, typeof Spicetify.Player.next)");
        assert!(p.is_ok());
        assert_eq!(logs(&ops), vec!["hi hello function"]);
    }

    #[test]
    fn infinite_loop_is_interrupted() {
        let started = Instant::now();
        let (p, _) = plugin("spin", &[], "while (true) {}");
        assert!(p.unwrap_err().contains("interrupted"));
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn freezing_callback_strikes_out() {
        let (p, _) = plugin("freeze", &[], "Rustify.on('songchange', () => { while (true) {} })");
        let mut p = p.unwrap();
        for _ in 0..MAX_STRIKES {
            assert!(p.call("__emit", "songchange".into(), Some("{}".into())).is_err());
        }
        assert!(p.strikes >= MAX_STRIKES);
    }

    #[test]
    fn memory_is_capped() {
        let (p, _) = plugin("hog", &[], "const a = []; while (true) a.push(new Array(1e6).fill(1));");
        let error = p.unwrap_err();
        assert!(error.contains("out of memory") || error.contains("interrupted"), "{error}");
    }

    #[test]
    fn permissions_are_enforced() {
        let (p, _) = plugin("noperm", &[], "Rustify.Player.next()");
        assert!(p.unwrap_err().contains("permission"));
        let (p, ops) = plugin("withperm", &["player"], "Rustify.Player.next()");
        assert!(p.is_ok());
        assert!(ops.lock().unwrap().iter().any(|o| matches!(o, UiOp::Player { action, .. } if action == "next")));
        let (p, _) = plugin("notheme", &[], "Rustify.Theme.set({ accent: '#ff0000' })");
        assert!(p.unwrap_err().contains("theme"));
        let (p, _) = plugin("nospotify", &[], "Rustify.Spotify.get('/v1/me')");
        assert!(p.is_ok());
    }

    #[test]
    fn spotify_proxy_rejects_bad_paths() {
        let (p, ops) = plugin(
            "proxy",
            &["spotify"],
            "Rustify.Spotify.get('https://evil.example/x').catch(e => console.error(e.message)); Rustify.Spotify.get('/v1/me').then(v => console.log('ok', v.ok));",
        );
        let mut p = p.unwrap();
        assert!(logs(&ops).iter().any(|l| l.contains("/v1/")));
        let id = p.shared.borrow().next_request;
        p.call_resolve(id, true, "{\"ok\":true}".into()).unwrap();
        assert!(logs(&ops).iter().any(|l| l == "ok true"));
    }

    #[test]
    fn no_ambient_authority() {
        let script = "console.log([typeof require, typeof process, typeof XMLHttpRequest, typeof __host, typeof __hostAsync, typeof std, typeof os, typeof WebSocket].join(','));
            fetch('https://example.com').catch(e => console.log('fetch ' + e.message));";
        let (p, ops) = plugin("sandbox", &[], script);
        assert!(p.is_ok());
        let l = logs(&ops);
        assert_eq!(l[0], "undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined");
        assert!(l[1].contains("\"network\" permission"), "{l:?}");
    }

    #[test]
    fn rustify_api_is_frozen() {
        let (p, ops) = plugin("frozen", &[], "'use strict'; try { Rustify.Player = 1 } catch (e) { console.log('blocked') }");
        assert!(p.is_ok());
        assert_eq!(logs(&ops), vec!["blocked"]);
    }

    #[test]
    fn storage_is_capped_and_isolated() {
        let (p, _) = plugin("store-a", &[], "for (let i = 0; i < 20; i++) Rustify.Storage.set('k' + i, 'x'.repeat(100000))");
        assert!(p.unwrap_err().contains("full"));
        let (p, ops) = plugin("store-b", &[], "Rustify.Storage.set('k', 'v'); console.log(Rustify.Storage.get('k'))");
        assert!(p.is_ok());
        assert_eq!(logs(&ops), vec!["v"]);
    }

    #[test]
    fn ui_spam_is_rate_limited() {
        let (p, _) = plugin("spam", &[], "for (let i = 0; i < 1000; i++) Rustify.showNotification('x'), console.log(i)");
        assert!(p.unwrap_err().contains("too many"));
    }

    #[test]
    fn bad_icons_and_uris_are_rejected() {
        assert!(valid_icon("M8 5.5 V18.5 L19 12 Z"));
        assert!(!valid_icon("<svg onload=alert(1)>"));
        assert!(!valid_icon("url(javascript:x)"));
        assert!(valid_uri("spotify:track:0s3nnoMeVWz3989MkNQiRf"));
        assert!(!valid_uri("spotify:user:x:collection"));
        assert!(!valid_uri("javascript:alert(1)"));
        assert!(valid_api_path("/v1/me/top/tracks?limit=10"));
        assert!(!valid_api_path("/v1/../../token"));
        let (p, _) = plugin("icon", &[], "Rustify.UI.Topbar.Button('x', '<script>', () => {})");
        assert!(p.is_err());
    }

    #[test]
    fn timers_fire() {
        let (p, ops) = plugin("timer", &[], "setTimeout(() => console.log('tick'), 10)");
        let mut p = p.unwrap();
        let id = *p.shared.borrow().timers.keys().next().unwrap();
        p.fire(id).unwrap();
        assert_eq!(logs(&ops), vec!["tick"]);
    }
    #[test]
    fn new_plugin_sees_current_state() {
        let ops = Arc::new(Mutex::new(Vec::new()));
        let seen = ops.clone();
        let sink: Sink = Arc::new(move |op| seen.lock().unwrap().push(op));
        let fetch: Fetch = Arc::new(|_, done| done(Ok("null".into())));
        let (tx, _rx) = mpsc::channel();
        let manifest = Manifest {
            id: "late".into(),
            name: "late".into(),
            version: String::new(),
            author: String::new(),
            description: String::new(),
            main: "index.js".into(),
            permissions: Vec::new(),
            theme: BTreeMap::new(),
            ..Manifest::default()
        };
        let spec = Spec {
            manifest,
            script: "console.log(Rustify.Player.data.track.name, Rustify.Player.isPlaying())".into(),
            storage_dir: std::env::temp_dir(),
            state: Some(r#"{"track":{"name":"Song","artists":[]},"isPlaying":true}"#.into()),
        };
        assert!(Instance::start(spec, sink, fetch, tx).is_ok());
        assert_eq!(logs(&ops), vec!["Song true"]);
    }

    #[test]
    fn example_plugins_run() {
        let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/plugins");
        for entry in std::fs::read_dir(&examples).unwrap() {
            let dir = entry.unwrap().path();
            let (manifest, script) = manifest::load(&dir).unwrap();
            if manifest.kind == manifest::Kind::Theme {
                assert!(script.is_empty() && !manifest.theme.is_empty());
                continue;
            }
            let ops = Arc::new(Mutex::new(Vec::new()));
            let seen = ops.clone();
            let sink: Sink = Arc::new(move |op| seen.lock().unwrap().push(op));
            let fetch: Fetch = Arc::new(|_, done| done(Ok("{\"items\":[]}".into())));
            let (tx, _rx) = mpsc::channel();
            let storage_dir = std::env::temp_dir().join(format!("rustify-example-test-{}", std::process::id()));
            let id = manifest.id.clone();
            let mut p = Instance::start(Spec { manifest, script, storage_dir, state: None }, sink, fetch, tx).unwrap_or_else(|e| panic!("{id}: {e}"));
            let state = r#"{"track":{"id":"a","uri":"spotify:track:0s3nnoMeVWz3989MkNQiRf","name":"Song","album":"Album","duration":1000,"image":"","artists":[{"id":"b","name":"Artist"}]},"isPlaying":true,"position":0,"duration":1000,"volume":0.5,"shuffle":false,"repeat":"off","context":null}"#;
            p.call("__emit", "state".into(), Some(state.into())).unwrap();
            p.call("__emit", "songchange".into(), Some(state.into())).unwrap();
            assert!(logs(&ops).iter().any(|l| l.contains("loaded") || l.contains("ready")), "{id}");
        }
    }

    #[test]
    fn spicetify_extension_runs_unchanged() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/plugins/spicetify-song-info");
        let (manifest, script) = manifest::load(&dir).unwrap();
        let ops = Arc::new(Mutex::new(Vec::new()));
        let seen = ops.clone();
        let sink: Sink = Arc::new(move |op| seen.lock().unwrap().push(op));
        let requested = Arc::new(Mutex::new(Vec::new()));
        let paths = requested.clone();
        let fetch: Fetch = Arc::new(move |request, done| {
            paths.lock().unwrap().push(request.to_string());
            done(Ok(r#"{"name":"Song","album":{"release_date":"2024-05-01"}}"#.into()))
        });
        let (tx, rx) = mpsc::channel();
        let storage_dir = std::env::temp_dir().join(format!("rustify-port-test-{}", std::process::id()));
        let mut p = Instance::start(Spec { manifest, script, storage_dir, state: None }, sink, fetch, tx).unwrap();

        assert!(logs(&ops).iter().any(|l| l == "Song info ready, shuffle is off"));
        let (button, menu) = {
            let ops = ops.lock().unwrap();
            let button = ops.iter().find_map(|o| match o {
                UiOp::Control { place, id, label, active, .. } if place == "topbar" && label == "Song info" && *active => Some(id.clone()),
                _ => None,
            });
            let menu = ops.iter().find_map(|o| match o {
                UiOp::Menu { id, label, remove: false, .. } if label == "Show release date" => Some(id.clone()),
                _ => None,
            });
            (button.expect("topbar button"), menu.expect("context menu item"))
        };

        let state = r#"{"track":{"id":"a","uri":"spotify:track:0s3nnoMeVWz3989MkNQiRf","name":"Song","album":"Album","duration":1000,"image":"","artists":[{"id":"b","name":"Artist"}]},"isPlaying":true,"position":0,"duration":1000,"volume":0.5,"shuffle":false,"repeat":"off","context":null}"#;
        p.call("__emit", "state".into(), Some(state.into())).unwrap();
        p.call("__emit", "songchange".into(), Some(state.into())).unwrap();
        let notes = |ops: &Arc<Mutex<Vec<UiOp>>>| ops.lock().unwrap().iter().filter_map(|o| if let UiOp::Notify(t) = o { Some(t.clone()) } else { None }).collect::<Vec<_>>();
        assert_eq!(notes(&ops), vec!["Song by Artist"]);

        p.shared.borrow_mut().last_notify = None;
        p.call("__action", menu, Some(r#"{"uri":"spotify:track:0s3nnoMeVWz3989MkNQiRf","type":"track","title":"Song"}"#.into())).unwrap();
        assert_eq!(requested.lock().unwrap().as_slice(), ["/v1/tracks/0s3nnoMeVWz3989MkNQiRf"]);
        let Ok(Cmd::Resolve { id, ok, json, .. }) = rx.try_recv() else { panic!("no response queued") };
        p.call_resolve(id, ok, json).unwrap();
        assert_eq!(notes(&ops).last().unwrap(), "Song came out on 2024-05-01");

        p.shared.borrow_mut().last_notify = None;
        p.call("__action", button, None).unwrap();
        assert_eq!(notes(&ops).last().unwrap(), "Song info turned off");
        assert_eq!(p.shared.borrow().storage.get("song-info:enabled").map(String::as_str), Some("false"));
    }

    #[test]
    fn ui_hooks_are_validated() {
        let (p, ops) = plugin(
            "hooks",
            &["player"],
            "const s = Rustify.UI.NowPlaying.Section('stats', 'Stats'); s.render([{ type: 'text', text: 'hi' }, { type: 'script', text: 'x' }]);
             Rustify.UI.hide('friends-button'); Rustify.Player.toggleLike();
             let bad = 0; try { Rustify.UI.hide('settings-page'); } catch (e) { bad++; }
             try { Rustify.UI.NowPlaying.Section('../x', 'X'); } catch (e) { bad++; }
             console.log('rejected ' + bad);",
        );
        assert!(p.is_ok());
        let ops = ops.lock().unwrap();
        assert!(ops.iter().any(|o| matches!(o, UiOp::Section { id, title, remove: false, .. } if id == "stats" && title == "Stats")));
        assert!(ops.iter().any(|o| matches!(o, UiOp::SectionRender { nodes, .. } if nodes.len() == 1)));
        assert!(ops.iter().any(|o| matches!(o, UiOp::Hide { element, hidden: true, .. } if element == "friends-button")));
        assert!(ops.iter().any(|o| matches!(o, UiOp::Player { action, .. } if action == "like")));
        assert!(ops.iter().any(|o| matches!(o, UiOp::Log { text, .. } if text == "rejected 2")));
    }

    #[test]
    fn hook_spam_is_rate_limited() {
        let (p, ops) = plugin(
            "spam",
            &["theme"],
            "const s = Rustify.UI.NowPlaying.Section('x', 'X');
             for (let i = 0; i < 5000; i++) {
                 try { s.render([{ type: 'text', text: String(i) }]); } catch (e) {}
                 try { Rustify.UI.hide('queue-button'); } catch (e) {}
                 try { Rustify.Theme.set({ radius: String(i % 25) }); } catch (e) {}
             }",
        );
        assert!(p.is_ok());
        let delivered = ops.lock().unwrap().iter().filter(|o| matches!(o, UiOp::Section { .. } | UiOp::SectionRender { .. } | UiOp::Hide { .. } | UiOp::Theme { .. })).count();
        assert!(delivered <= UI_OPS_PER_SECOND as usize, "{delivered}");
        let (p, _) = plugin("bad-radius", &["theme"], "Rustify.Theme.set({ radius: '9999' })");
        assert!(p.unwrap_err().contains("radius"));
    }

    #[test]
    fn home_sections_and_track_row_buttons() {
        let (p, ops) = plugin(
            "rows",
            &[],
            "Rustify.UI.Home.Section('welcome', 'Welcome back').render([{ type: 'text', text: 'hi' }]);
             Rustify.UI.TrackRow.Button('Mark', 'star', (button, track) => console.log('row ' + track.uri + ' ' + track.title));",
        );
        let mut p = p.unwrap();
        let row = {
            let ops = ops.lock().unwrap();
            assert!(ops.iter().any(|o| matches!(o, UiOp::Section { place, id, .. } if place == "home" && id == "welcome")));
            ops.iter()
                .find_map(|o| match o {
                    UiOp::Control { place, id, .. } if place == "trackrow" => Some(id.clone()),
                    _ => None,
                })
                .expect("track row button")
        };
        p.call("__action", row, Some(r#"{"uri":"spotify:track:0s3nnoMeVWz3989MkNQiRf","type":"track","title":"Song"}"#.into())).unwrap();
        assert!(logs(&ops).iter().any(|l| l == "row spotify:track:0s3nnoMeVWz3989MkNQiRf Song"));
        let (p, _) = plugin("bad-place", &[], "Rustify.UI.NowPlaying.Section('ok', 'x'); globalThis.__x = 1;");
        assert!(p.is_ok());
    }

    #[test]
    fn row_and_home_hooks_are_hardened() {
        let (p, ops) = plugin(
            "row-spam",
            &[],
            "let bad = 0;
             try { Rustify.UI.TrackRow.Button('x', '<svg onload=alert(1)>', () => {}); } catch (e) { bad++; }
             try { Rustify.UI.Home.Section('../evil', 'X'); } catch (e) { bad++; }
             const h = Rustify.UI.Home.Section('h', 'H');
             try { h.render(Array.from({ length: 500 }, () => ({ type: 'text', text: 'x' }))); } catch (e) { bad++; }
             console.log('bad ' + bad);
             for (let i = 0; i < 5000; i++) {
                 try { Rustify.UI.TrackRow.Button('b' + i, 'star', () => {}); } catch (e) {}
                 try { h.render([{ type: 'text', text: String(i) }]); } catch (e) {}
             }",
        );
        if let Err(e) = &p {
            panic!("{e}");
        }
        let ops = ops.lock().unwrap();
        let delivered = ops.iter().filter(|o| matches!(o, UiOp::Control { .. } | UiOp::Section { .. } | UiOp::SectionRender { .. })).count();
        assert!(delivered <= UI_OPS_PER_SECOND as usize, "{delivered}");
        assert!(ops.iter().all(|o| !matches!(o, UiOp::SectionRender { nodes, .. } if nodes.len() > 100)));
        assert!(ops.iter().any(|o| matches!(o, UiOp::Log { text, .. } if text == "bad 2")));
        let rows = ops.iter().filter(|o| matches!(o, UiOp::Control { place, .. } if place == "trackrow")).count();
        assert!(rows <= UI_OPS_PER_SECOND as usize);
    }

    #[test]
    fn spicetify_popup_react_and_clipboard() {
        let script = r##"
            (function ready() {
                if (!window.Spicetify?.PopupModal || !Spicetify.React || !Spicetify.Platform?.ClipboardAPI) {
                    setTimeout(ready, 300);
                    return;
                }
                if ((Spicetify.Config?.custom_apps || []).includes("x")) return;
                const h = Spicetify.React.createElement;
                const windows = /windows/i.test(navigator.userAgent);
                function Body() {
                    const [copied, setCopied] = Spicetify.React.useState(false);
                    return h("div", { style: { display: "flex" } },
                        h("p", null, "Hello ", h("a", { href: "https://example.com" }, "there"), windows ? " on Windows" : ""),
                        h("div", null, h("code", null, "some command"),
                            h("button", { style: { background: "#fff" }, onClick: () => { Spicetify.Platform.ClipboardAPI.copy("some command"); setCopied(true); } }, copied ? "Copied" : "Copy")),
                        h("button", { style: { background: "transparent" }, onClick: () => { localStorage.setItem("snooze", "1"); Spicetify.PopupModal.hide(); } }, "Later"));
                }
                Spicetify.PopupModal.display({ title: "Moved", content: h(Body) });
            })();
        "##;
        let (p, ops) = plugin("popup", &[], script);
        let mut p = p.unwrap();
        let renders = |ops: &Arc<Mutex<Vec<UiOp>>>| -> Vec<Vec<Value>> {
            ops.lock().unwrap().iter().filter_map(|o| if let UiOp::SectionRender { nodes, .. } = o { Some(nodes.clone()) } else { None }).collect()
        };
        let (key, handle) = {
            let ops = ops.lock().unwrap();
            ops.iter()
                .find_map(|o| match o {
                    UiOp::Section { place, id, title, handle, remove: false, .. } if place == "dialog" && title == "Moved" => Some((id.clone(), handle.clone())),
                    _ => None,
                })
                .expect("dialog")
        };
        let first = renders(&ops).last().cloned().unwrap();
        let texts: Vec<&str> = first.iter().filter_map(|n| n["text"].as_str()).collect();
        assert_eq!(texts, ["Hello there on Windows", "some command", "Copy", "Later"]);
        assert_eq!(first[2]["primary"], true);
        assert_eq!(first[3]["primary"], false);

        p.call("__action", handle.clone(), Some(r#"{"id":"r0","value":""}"#.into())).unwrap();
        assert!(ops.lock().unwrap().iter().any(|o| matches!(o, UiOp::Clipboard { text, .. } if text == "some command")));
        assert_eq!(renders(&ops).last().unwrap()[2]["text"], "Copied");

        p.call("__action", handle, Some(r#"{"id":"r1","value":""}"#.into())).unwrap();
        assert_eq!(p.shared.borrow().storage.get("snooze").map(String::as_str), Some("1"));
        assert!(ops.lock().unwrap().iter().any(|o| matches!(o, UiOp::Section { id, remove: true, .. } if *id == key)));
    }

    type NetHarness = (Result<Instance, String>, Arc<Mutex<Vec<UiOp>>>, Arc<Mutex<Vec<String>>>, mpsc::Receiver<Cmd>);

    fn net_plugin(perms: &[&str], script: &str) -> NetHarness {
        let ops = Arc::new(Mutex::new(Vec::new()));
        let seen = ops.clone();
        let sink: Sink = Arc::new(move |op| seen.lock().unwrap().push(op));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let fetch: Fetch = Arc::new(move |request, done| {
            let headers = match &request {
                Request::Net { headers, .. } => headers.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(";"),
                Request::Spotify(_) => String::new(),
            };
            log.lock().unwrap().push(format!("{request} [{headers}]"));
            done(Ok(r#"{"status":200,"statusText":"OK","url":"https://api.example.com/songs","headers":{"content-type":"application/json"},"body":"{\"songs\":[1,2]}"}"#.into()))
        });
        let (tx, rx) = mpsc::channel();
        let manifest = Manifest {
            id: "net".into(),
            name: "net".into(),
            main: "index.js".into(),
            permissions: perms.iter().map(|p| p.to_string()).collect(),
            network: vec!["example.com".into()],
            ..Manifest::default()
        };
        let storage_dir = std::env::temp_dir().join(format!("rustify-net-test-{}", std::process::id()));
        (Instance::start(Spec { manifest, script: script.into(), storage_dir, state: None }, sink, fetch, tx), ops, requests, rx)
    }

    #[test]
    fn network_needs_permission_and_listed_hosts() {
        let (p, ops, requests, _rx) = net_plugin(&[], "fetch('https://api.example.com/a').catch(e => console.log('blocked ' + e.message))");
        assert!(p.is_ok());
        assert!(requests.lock().unwrap().is_empty());
        assert!(logs(&ops).iter().any(|l| l.contains("blocked") && l.contains("network")));

        let script = "let out = [];
            for (const url of ['https://evil.test/x', 'http://api.example.com/x', 'https://example.com.evil.test/x', 'https://api.example.com:444/x']) {
                fetch(url).catch(() => out.push('no'));
            }
            fetch('https://api.example.com/songs', { method: 'POST', headers: { 'X-Key': 'a', Cookie: 'steal', Host: 'evil' }, body: { q: 1 } })
                .then(r => r.json().then(j => console.log('ok ' + r.status + ' ' + r.headers.get('Content-Type') + ' ' + j.songs.length)));
            Promise.resolve().then(() => Promise.resolve()).then(() => console.log('blocked ' + out.length));";
        let (p, ops, requests, rx) = net_plugin(&["network"], script);
        let mut p = p.unwrap();
        assert_eq!(requests.lock().unwrap().as_slice(), ["POST https://api.example.com/songs [x-key=a]"]);
        while let Ok(Cmd::Resolve { id, ok, json, .. }) = rx.try_recv() {
            p.call_resolve(id, ok, json).unwrap();
        }
        let l = logs(&ops);
        assert!(l.iter().any(|x| x == "blocked 4"), "{l:?}");
        assert!(l.iter().any(|x| x == "ok 200 application/json 2"), "{l:?}");
    }

    #[test]
    fn audio_needs_permission_and_listed_hosts() {
        let (p, ops, _, _) = net_plugin(&["network"], "try { Rustify.Audio.load({ url: 'https://cdn.example.com/a.mp3' }); } catch (e) { console.log('no audio'); }");
        assert!(p.is_ok());
        assert_eq!(logs(&ops), vec!["no audio"]);

        let script = "let bad = 0;
            try { Rustify.Audio.load({ url: 'https://evil.test/a.mp3' }); } catch (e) { bad++; }
            try { Rustify.Audio.load({ url: 'http://cdn.example.com/a.mp3' }); } catch (e) { bad++; }
            Rustify.Audio.load({ url: 'https://cdn.example.com/a.mp3', title: 'Song', artist: 'Artist', image: 'https://evil.test/x.jpg' });
            const el = new Audio('https://cdn.example.com/b.mp3');
            el.title = 'Two';
            el.play();
            Rustify.Audio.on('next', e => console.log('next ' + e.event));
            console.log('bad ' + bad);";
        let (p, ops, _, _) = net_plugin(&["audio"], script);
        let mut p = p.unwrap();
        let loads: Vec<Value> = ops.lock().unwrap().iter().filter_map(|o| if let UiOp::Audio { payload, .. } = o { Some(payload.clone()) } else { None }).collect();
        assert_eq!(loads.len(), 2);
        assert_eq!(loads[0]["track"]["title"], "Song");
        assert_eq!(loads[0]["track"]["image"], "");
        assert_eq!(loads[1]["track"]["url"], "https://cdn.example.com/b.mp3");
        p.call("__emit", "audio".into(), Some(r#"{"event":"next","message":""}"#.into())).unwrap();
        let l = logs(&ops);
        assert!(l.iter().any(|x| x == "bad 2"), "{l:?}");
        assert!(l.iter().any(|x| x == "next next"), "{l:?}");
    }

    #[test]
    fn react_pages_search_and_queue() {
        let script = r##"
            const h = Spicetify.React.createElement;
            const page = Rustify.UI.Page("browse", "Browse", "list");
            function Browse() {
                const [picked, setPicked] = Spicetify.React.useState("none");
                return h("div", null,
                    h("h2", null, "Eras"),
                    h("div", { onClick: () => setPicked("era-1") }, h("img", { src: "https://i.scdn.co/image/abc" }), h("span", null, "Era One"), h("span", null, "12 songs")),
                    h("p", null, "Picked " + picked));
            }
            page.render(h(Browse));
            page.onAction(id => console.log("plain " + id));
            const results = Rustify.UI.Search.Section("vault", "From the vault");
            Rustify.on("search", ({ query }) => results.render([{ type: "row", id: "s1", text: "Result for " + query }]));
            Rustify.Audio.setQueue([{ id: "a", title: "Next song", artist: "Artist" }], "Up next");
            Rustify.Audio.on("select", e => console.log("select " + e.id));
            for (let i = 0; i < 15; i++) Rustify.UI.Page("p" + i, "P" + i, "list");
        "##;
        let (p, ops, _, _) = net_plugin(&["audio"], script);
        let mut p = p.unwrap();
        let (page_handle, queue_handle) = {
            let ops = ops.lock().unwrap();
            let page = ops.iter().find_map(|o| match o {
                UiOp::Page { id, handle, .. } if id == "browse" => Some(handle.clone()),
                _ => None,
            });
            let queue = ops.iter().find_map(|o| match o {
                UiOp::Section { place, handle, .. } if place == "queue" => Some(handle.clone()),
                _ => None,
            });
            assert!(ops.iter().any(|o| matches!(o, UiOp::Section { place, title, .. } if place == "search" && title == "From the vault")));
            assert_eq!(ops.iter().filter(|o| matches!(o, UiOp::Page { remove: false, .. })).count(), 16);
            (page.unwrap(), queue.unwrap())
        };
        let page_nodes = |ops: &Arc<Mutex<Vec<UiOp>>>| -> Vec<Value> {
            ops.lock().unwrap().iter().rev().find_map(|o| if let UiOp::Render { nodes, .. } = o { Some(nodes.clone()) } else { None }).unwrap()
        };
        let first = page_nodes(&ops);
        assert_eq!(first[0]["type"], "heading");
        assert_eq!(first[1]["type"], "row");
        assert_eq!(first[1]["text"], "Era One");
        assert_eq!(first[1]["sub"], "12 songs");
        assert_eq!(first[1]["image"], "https://i.scdn.co/image/abc");
        assert_eq!(first[2]["text"], "Picked none");

        p.call("__action", page_handle.clone(), Some(r#"{"id":"r0","value":""}"#.into())).unwrap();
        assert_eq!(page_nodes(&ops)[2]["text"], "Picked era-1");
        p.call("__action", page_handle, Some(r#"{"id":"custom","value":""}"#.into())).unwrap();

        p.call("__emit", "search".into(), Some(r#"{"query":"lucid","filter":""}"#.into())).unwrap();
        assert!(ops.lock().unwrap().iter().any(|o| matches!(o, UiOp::SectionRender { nodes, .. } if nodes[0]["text"] == "Result for lucid")));

        p.call("__action", queue_handle, Some(r#"{"id":"q:a","value":""}"#.into())).unwrap();
        let l = logs(&ops);
        assert!(l.iter().any(|x| x == "plain custom"), "{l:?}");
        assert!(l.iter().any(|x| x == "select a"), "{l:?}");
    }

    #[test]
    fn authorization_header_reaches_listed_hosts() {
        let script = "fetch('https://api.example.com/me', { headers: { Authorization: 'Bearer abc', Cookie: 'x' } })";
        let (p, ops, requests, _) = net_plugin(&["network"], script);
        assert!(p.is_ok());
        assert_eq!(requests.lock().unwrap().as_slice(), ["GET https://api.example.com/me [authorization=Bearer abc]"]);
        drop(ops);
    }
}
