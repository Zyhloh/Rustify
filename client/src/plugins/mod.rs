mod host;
mod install;
mod library;
pub mod manifest;
mod review;
#[cfg(test)]
mod bench;

use crate::{App, HiddenUi, MenuEntry, Player, PluginBlock, PluginControl, PluginInfo, PluginLog, PluginPageLink, PluginSection, Plugins, Store, Theme, config::Config, ctx::Ctx, pages};
use host::{Cmd, UiOp};
use manifest::Kind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, mpsc},
};

pub use library::{
    cancel_install_link, dropped, hover_drop, install_file, install_link, open_link, open_plugin_folder, select, set_filter, set_query, set_tab, uninstall,
    use_default_theme,
};

const MAX_LOGS: usize = 300;

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    system: bool,
    #[serde(default)]
    enabled: BTreeSet<String>,
    #[serde(default)]
    dev_mode: bool,
}

#[derive(Clone)]
struct Found {
    manifest: Option<manifest::Manifest>,
    script: String,
    folder: String,
    error: String,
    review: review::Report,
    icon: Option<slint::Image>,
}

struct Menu {
    plugin: String,
    id: String,
    label: String,
    types: Vec<String>,
}

#[derive(Default)]
struct State {
    tx: Option<mpsc::Sender<Cmd>>,
    ctx: Option<Ctx>,
    found: Vec<Found>,
    saved: Saved,
    watching: bool,
    running: BTreeSet<String>,
    errors: BTreeMap<String, String>,
    menus: Vec<Menu>,
    pages: BTreeMap<(String, String), (String, String, String)>,
    blocks: BTreeMap<(String, String), Vec<PluginBlock>>,
    sections: BTreeMap<(String, String), (String, String, Vec<PluginBlock>)>,
    hidden: BTreeMap<String, BTreeSet<String>>,
    themes: BTreeMap<String, BTreeMap<String, String>>,
    defaults: BTreeMap<String, Color>,
    default_radius: f32,
    open_page: Option<(String, String)>,
    last_state: Option<String>,
    last_track: String,
    last_playing: Option<bool>,
    query: String,
    tab: String,
    filter: String,
    selected: Option<String>,
    topbar: Rc<VecModel<PluginControl>>,
    playbar: Rc<VecModel<PluginControl>>,
    page_links: Rc<VecModel<PluginPageLink>>,
    block_model: Rc<VecModel<PluginBlock>>,
    section_model: Rc<VecModel<PluginSection>>,
    logs: Rc<VecModel<PluginLog>>,
    shown: Rc<VecModel<PluginInfo>>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn dir() -> PathBuf {
    Config::data_dir().join("plugins")
}

fn data_dir() -> PathBuf {
    Config::data_dir().join("plugin-data")
}

fn staging_dir() -> PathBuf {
    Config::data_dir().join("plugin-staging")
}

fn saved_path() -> PathBuf {
    Config::data_dir().join("plugins.json")
}

fn send(cmd: Cmd) {
    STATE.with(|s| {
        if let Some(tx) = &s.borrow().tx {
            let _ = tx.send(cmd);
        }
    });
}

fn system_on() -> bool {
    STATE.with(|s| s.borrow().saved.system)
}

fn scan() -> Vec<Found> {
    let root = dir();
    let _ = std::fs::create_dir_all(&root);
    let mut found: Vec<Found> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !e.file_name().to_string_lossy().starts_with('.'))
        .take(200)
        .map(|entry| {
            let folder = entry.file_name().to_string_lossy().into_owned();
            match manifest::load(&entry.path()) {
                Ok((m, script)) => {
                    let review = review::review(&m, &script);
                    let icon = (!m.icon.is_empty()).then(|| library::load_icon(&entry.path().join(&m.icon))).flatten();
                    Found { manifest: Some(m), script, folder, error: String::new(), review, icon }
                }
                Err(error) => Found { manifest: None, script: String::new(), folder, error, review: review::Report::default(), icon: None },
            }
        })
        .collect();
    found.sort_by_key(|f| f.manifest.as_ref().map(|m| m.name.to_lowercase()).unwrap_or_else(|| f.folder.clone()));
    found
}

pub fn install(app: &App, ctx: &Ctx) {
    let weak = app.as_weak();
    let sink: host::Sink = Arc::new(move |op: UiOp| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                apply(&app, op);
            }
        });
    });
    let api = ctx.api.clone();
    let rt = ctx.rt.clone();
    let fetch: host::Fetch = Arc::new(move |path, done| {
        let api = api.clone();
        rt.spawn(async move {
            let url = format!("/plugin/spotify?path={}", crate::ctx::url_encode(&path));
            let result = api.get::<Value>(&url).await.map(|v| v.to_string()).map_err(|e| e.to_string());
            done(result);
        });
    });
    let tx = host::spawn(sink, fetch);

    let theme = app.global::<Theme>();
    let defaults: BTreeMap<String, Color> = manifest::THEME_KEYS.iter().map(|k| (k.to_string(), theme_get(&theme, k))).collect();
    let saved: Saved = std::fs::read(saved_path()).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default();
    let _ = std::fs::remove_dir_all(staging_dir());

    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.tx = Some(tx);
        s.ctx = Some(ctx.clone());
        s.defaults = defaults;
        s.default_radius = theme.get_radius();
        s.saved = saved;
        s.tab = "plugins".into();
        s.filter = "all".into();
        let plugins = app.global::<Plugins>();
        plugins.set_topbar(ModelRc::from(s.topbar.clone()));
        plugins.set_playbar(ModelRc::from(s.playbar.clone()));
        plugins.set_pages(ModelRc::from(s.page_links.clone()));
        plugins.set_blocks(ModelRc::from(s.block_model.clone()));
        plugins.set_sections(ModelRc::from(s.section_model.clone()));
        plugins.set_logs(ModelRc::from(s.logs.clone()));
        plugins.set_shown(ModelRc::from(s.shown.clone()));
        plugins.set_dev_mode(s.saved.dev_mode);
        plugins.set_system(s.saved.system);
        plugins.set_folder(dir().display().to_string().into());
    });
    reload(app);
}

pub fn set_system(app: &App, on: bool) {
    STATE.with(|s| s.borrow_mut().saved.system = on);
    save();
    app.global::<Plugins>().set_system(on);
    if !on && matches!(pages::current(), pages::Route::Extensions | pages::Route::Extension { .. } | pages::Route::Plugin { .. }) {
        pages::push(app, pages::Route::Settings);
    }
    reload(app);
}

pub fn reload(app: &App) {
    let found = scan();
    let (enabled, on) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        for id in s.running.clone() {
            if let Some(tx) = &s.tx {
                let _ = tx.send(Cmd::Unload(id.clone()));
            }
            clear_plugin(&mut s, &id);
        }
        s.running.clear();
        s.errors.clear();
        s.themes.clear();
        s.found = found.clone();
        (s.saved.enabled.clone(), s.saved.system)
    });
    reset_theme(app);
    if on {
        for f in found {
            if let Some(m) = f.manifest
                && enabled.contains(&m.id)
            {
                activate(app, m, f.script);
            }
        }
        watch(app);
    }
    refresh(app);
}

fn activate(app: &App, m: manifest::Manifest, script: String) {
    match m.kind {
        Kind::Plugin => start(m, script),
        Kind::Theme => {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.running.insert(m.id.clone());
                s.themes.insert(m.id.clone(), m.theme.clone());
            });
            reapply_themes(app);
        }
    }
}

fn deactivate(id: &str) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.running.remove(id);
        s.errors.remove(id);
        clear_plugin(&mut s, id);
    });
    send(Cmd::Unload(id.to_owned()));
}

pub fn sync(app: &App) {
    if !system_on() {
        return;
    }
    let found = scan();
    let mut stop = Vec::new();
    let mut launch = Vec::new();
    let mut added = Vec::new();
    let mut needs_approval = Vec::new();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let old: BTreeMap<String, Found> = s.found.iter().map(|f| (f.folder.clone(), f.clone())).collect();
        for f in &found {
            let Some(m) = &f.manifest else {
                if let Some(prev) = old.get(&f.folder).and_then(|o| o.manifest.as_ref())
                    && s.running.contains(&prev.id)
                {
                    stop.push(prev.id.clone());
                }
                continue;
            };
            let before = old.get(&f.folder);
            let prev = before.and_then(|o| o.manifest.as_ref());
            if before.is_none() {
                added.push(m.name.clone());
            }
            if !s.saved.enabled.contains(&m.id) {
                continue;
            }
            let unchanged = prev == Some(m) && before.is_some_and(|o| o.script == f.script);
            if unchanged && (s.running.contains(&m.id) || s.errors.contains_key(&m.id)) {
                continue;
            }
            let widened = prev.is_some_and(|p| m.permissions.iter().any(|x| !p.permissions.contains(x)));
            let riskier = before.is_some_and(|o| f.review.level() > o.review.level());
            if widened || riskier {
                needs_approval.push((m.id.clone(), widened));
                stop.push(m.id.clone());
                continue;
            }
            if s.running.contains(&m.id) {
                stop.push(m.id.clone());
            }
            launch.push((m.clone(), f.script.clone()));
        }
        let live: BTreeSet<String> = found.iter().filter_map(|f| f.manifest.as_ref().map(|m| m.id.clone())).collect();
        for id in s.running.iter().filter(|id| !live.contains(*id)) {
            stop.push(id.clone());
        }
        for (id, widened) in &needs_approval {
            s.saved.enabled.remove(id);
            let why = if *widened { "asks for new permissions" } else { "has new safety warnings" };
            log(&s, id, "warn", &format!("was turned off because the update {why}. Turn it on again to approve"));
        }
        s.found = found;
    });
    if !needs_approval.is_empty() {
        save();
    }
    for id in &stop {
        deactivate(id);
    }
    if !stop.is_empty() {
        reset_theme(app);
        reapply_themes(app);
    }
    for (m, script) in launch {
        activate(app, m, script);
    }
    match added.as_slice() {
        [] => {}
        [one] => crate::menu::toast(app, &format!("Found {one}. Turn it on in Plugins & themes")),
        many => crate::menu::toast(app, &format!("Found {} new add-ons. Turn them on in Plugins & themes", many.len())),
    }
    if let Some((id, _)) = needs_approval.first() {
        crate::menu::toast(app, &format!("{id} was updated and needs your approval again"));
    }
    refresh(app);
}

fn fingerprint() -> Vec<(String, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir()).into_iter().flatten().flatten().take(200) {
        let folder = entry.file_name().to_string_lossy().into_owned();
        out.push((folder.clone(), 0, std::time::UNIX_EPOCH));
        for file in std::fs::read_dir(entry.path()).into_iter().flatten().flatten().take(50) {
            if let Ok(meta) = file.metadata()
                && meta.is_file()
            {
                out.push((format!("{folder}/{}", file.file_name().to_string_lossy()), meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH)));
            }
        }
    }
    out.sort();
    out
}

fn watch(app: &App) {
    let started = STATE.with(|s| std::mem::replace(&mut s.borrow_mut().watching, true));
    if started {
        return;
    }
    let weak = app.as_weak();
    let _ = std::thread::Builder::new().name("rustify-plugin-watch".into()).spawn(move || {
        let mut applied = fingerprint();
        let mut pending: Option<Vec<_>> = None;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(1000));
            let now = fingerprint();
            if now == applied {
                pending = None;
                continue;
            }
            if pending.as_ref() != Some(&now) {
                pending = Some(now);
                continue;
            }
            applied = now;
            pending = None;
            let weak = weak.clone();
            if slint::invoke_from_event_loop(move || {
                if let Some(app) = weak.upgrade() {
                    sync(&app);
                }
            })
            .is_err()
            {
                break;
            }
        }
    });
}

fn start(manifest: manifest::Manifest, script: String) {
    send(Cmd::Load(Box::new(host::Spec { manifest, script, storage_dir: data_dir(), state: STATE.with(|s| s.borrow().last_state.clone()) })));
}

fn save() {
    STATE.with(|s| {
        if let Ok(raw) = serde_json::to_vec_pretty(&s.borrow().saved) {
            let _ = std::fs::write(saved_path(), raw);
        }
    });
}

pub struct EnableAsk {
    pub title: String,
    pub body: String,
    pub notes: Vec<String>,
    pub danger: bool,
}

pub fn pending_enable(id: &str) -> Option<EnableAsk> {
    STATE.with(|s| {
        let s = s.borrow();
        let f = s.found.iter().find(|f| f.manifest.as_ref().is_some_and(|m| m.id == id))?;
        let m = f.manifest.as_ref()?;
        let author = if m.author.is_empty() { "an unknown author".to_owned() } else { m.author.clone() };
        let level = f.review.level();
        let notes: Vec<String> = f.review.findings.iter().filter(|x| x.level > review::Level::Note).map(|x| x.text.clone()).collect();
        let body = match m.kind {
            Kind::Theme => format!("{} by {author} will change Rustify's colors.", m.name),
            Kind::Plugin if m.permissions.is_empty() => format!(
                "{} by {author} will run inside Rustify. It can add buttons, menu items and pages, but doesn't ask for any special permissions. Only turn on plugins you trust.",
                m.name
            ),
            Kind::Plugin => format!(
                "{} by {author} will run inside Rustify. It can {}. Only turn on plugins you trust.",
                m.name,
                library::lower_first(&library::permissions_text(&m.permissions))
            ),
        };
        let title = match level {
            Some(review::Level::Caution) => format!("{} may be harmful", m.name),
            _ => format!("Turn on {}?", m.name),
        };
        Some(EnableAsk { title, body, notes, danger: level == Some(review::Level::Caution) })
    })
}

pub fn needs_prompt(id: &str) -> bool {
    STATE.with(|s| {
        s.borrow().found.iter().filter_map(|f| f.manifest.as_ref()).any(|m| m.id == id && m.kind == Kind::Plugin)
    })
}

pub fn set_enabled(app: &App, id: &str, enabled: bool) {
    let target = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let found = s.found.iter().find(|f| f.manifest.as_ref().is_some_and(|m| m.id == id)).cloned();
        if enabled {
            s.saved.enabled.insert(id.to_owned());
        } else {
            s.saved.enabled.remove(id);
        }
        found
    });
    let is_theme = target.as_ref().and_then(|f| f.manifest.as_ref()).is_some_and(|m| m.kind == Kind::Theme);
    if enabled && is_theme {
        let others: Vec<String> = STATE.with(|s| {
            let s = s.borrow();
            s.found
                .iter()
                .filter_map(|f| f.manifest.as_ref())
                .filter(|m| m.kind == Kind::Theme && m.id != id && s.saved.enabled.contains(&m.id))
                .map(|m| m.id.clone())
                .collect()
        });
        for other in others {
            STATE.with(|s| s.borrow_mut().saved.enabled.remove(&other));
            deactivate(&other);
        }
        reset_theme(app);
        reapply_themes(app);
    }
    save();
    if enabled {
        if system_on()
            && let Some((m, script)) = target.and_then(|f| f.manifest.map(|m| (m, f.script)))
        {
            activate(app, m, script);
        }
    } else {
        deactivate(id);
        reset_theme(app);
        reapply_themes(app);
    }
    refresh(app);
}

pub fn set_dev_mode(on: bool) {
    STATE.with(|s| s.borrow_mut().saved.dev_mode = on);
    save();
}

fn clear_plugin(s: &mut State, id: &str) {
    retain(&s.topbar, |c| c.plugin != id);
    retain(&s.playbar, |c| c.plugin != id);
    retain(&s.page_links, |p| p.plugin != id);
    s.menus.retain(|m| m.plugin != id);
    s.pages.retain(|(p, _), _| p != id);
    s.blocks.retain(|(p, _), _| p != id);
    s.themes.remove(id);
    if s.sections.keys().any(|(p, _)| p == id) {
        s.sections.retain(|(p, _), _| p != id);
        rebuild_sections(s);
    }
    s.hidden.remove(id);
}

fn rebuild_sections(s: &State) {
    let rows: Vec<PluginSection> = s
        .sections
        .iter()
        .map(|((plugin, id), (title, _, blocks))| PluginSection {
            plugin: plugin.as_str().into(),
            id: id.as_str().into(),
            title: title.as_str().into(),
            blocks: ModelRc::from(Rc::new(VecModel::from(blocks.clone()))),
        })
        .collect();
    s.section_model.set_vec(rows);
}

fn sync_hidden(app: &App) {
    let hidden = STATE.with(|s| {
        let s = s.borrow();
        let set: BTreeSet<&str> = s.hidden.values().flatten().map(String::as_str).collect();
        HiddenUi {
            friends: set.contains("friends-button"),
            news: set.contains("news-button"),
            now_playing: set.contains("now-playing-button"),
            lyrics: set.contains("lyrics-button"),
            queue: set.contains("queue-button"),
            devices: set.contains("devices-button"),
            mini_player: set.contains("mini-player-button"),
            fullscreen: set.contains("fullscreen-button"),
        }
    });
    let plugins = app.global::<Plugins>();
    if plugins.get_hidden() != hidden {
        plugins.set_hidden(hidden);
    }
}

fn retain<T: Clone + 'static>(model: &VecModel<T>, keep: impl Fn(&T) -> bool) {
    let rows: Vec<T> = model.iter().filter(|r| keep(r)).collect();
    if rows.len() != model.row_count() {
        model.set_vec(rows);
    }
}

fn refresh(app: &App) {
    library::refresh(app);
    sync_hidden(app);
}

fn theme_get(theme: &Theme, key: &str) -> Color {
    match key {
        "base" => theme.get_base(),
        "surface" => theme.get_surface(),
        "surface-raised" => theme.get_surface_raised(),
        "surface-high" => theme.get_surface_high(),
        "tile" => theme.get_tile(),
        "tile-hover" => theme.get_tile_hover(),
        "hover" => theme.get_hover(),
        "press" => theme.get_press(),
        "text" => theme.get_text(),
        "text-dim" => theme.get_text_dim(),
        "text-faint" => theme.get_text_faint(),
        "accent" => theme.get_accent(),
        "accent-hover" => theme.get_accent_hover(),
        "danger" => theme.get_danger(),
        _ => theme.get_track(),
    }
}

fn theme_set(theme: &Theme, key: &str, color: Color) {
    match key {
        "base" => theme.set_base(color),
        "surface" => theme.set_surface(color),
        "surface-raised" => theme.set_surface_raised(color),
        "surface-high" => theme.set_surface_high(color),
        "tile" => theme.set_tile(color),
        "tile-hover" => theme.set_tile_hover(color),
        "hover" => theme.set_hover(color),
        "press" => theme.set_press(color),
        "text" => theme.set_text(color),
        "text-dim" => theme.set_text_dim(color),
        "text-faint" => theme.set_text_faint(color),
        "accent" => theme.set_accent(color),
        "accent-hover" => theme.set_accent_hover(color),
        "danger" => theme.set_danger(color),
        "track" => theme.set_track(color),
        _ => {}
    }
}

pub fn parse_color(hex: &str) -> Option<Color> {
    if !manifest::valid_color(hex) {
        return None;
    }
    let raw = &hex[1..];
    let byte = |i: usize| u8::from_str_radix(&raw[i..i + 2], 16).ok();
    let alpha = if raw.len() == 8 { byte(6)? } else { 255 };
    Some(Color::from_argb_u8(alpha, byte(0)?, byte(2)?, byte(4)?))
}

fn reset_theme(app: &App) {
    let theme = app.global::<Theme>();
    STATE.with(|s| {
        let s = s.borrow();
        for (key, color) in &s.defaults {
            theme_set(&theme, key, *color);
        }
        theme.set_radius(s.default_radius);
    });
}

fn reapply_themes(app: &App) {
    let theme = app.global::<Theme>();
    STATE.with(|s| {
        let s = s.borrow();
        let kind = |id: &str| s.found.iter().filter_map(|f| f.manifest.as_ref()).find(|m| m.id == id).map(|m| m.kind);
        let mut ordered: Vec<(&String, &BTreeMap<String, String>)> = s.themes.iter().collect();
        ordered.sort_by_key(|(id, _)| kind(id) != Some(Kind::Theme));
        for (_, colors) in ordered {
            for (key, value) in colors {
                if key == "radius" {
                    if let Some(r) = manifest::radius(value) {
                        theme.set_radius(r);
                    }
                } else if let Some(color) = parse_color(value) {
                    theme_set(&theme, key, color);
                }
            }
        }
    });
}

fn log(s: &State, plugin: &str, level: &str, text: &str) {
    match level {
        "error" => tracing::warn!("plugin {plugin}: {text}"),
        _ => tracing::info!("plugin {plugin}: {text}"),
    }
    if s.logs.row_count() >= MAX_LOGS {
        s.logs.remove(0);
    }
    s.logs.push(PluginLog { plugin: plugin.into(), level: level.into(), text: text.into() });
}

fn control(plugin: &str, id: &str, label: &str, icon: &str, active: bool) -> PluginControl {
    PluginControl { plugin: plugin.into(), id: id.into(), label: label.into(), icon: icon.into(), active }
}

fn upsert(model: &VecModel<PluginControl>, row: PluginControl, cap: usize) -> bool {
    match model.iter().position(|c| c.plugin == row.plugin && c.id == row.id) {
        Some(i) => {
            if model.row_data(i).as_ref() != Some(&row) {
                model.set_row_data(i, row);
            }
            true
        }
        None if model.iter().filter(|c| c.plugin == row.plugin).count() < cap => {
            model.push(row);
            true
        }
        None => false,
    }
}

fn block(node: &Value) -> PluginBlock {
    let text = |k: &str| SharedString::from(node.get(k).and_then(Value::as_str).unwrap_or_default());
    let num = |k: &str, d: f64| node.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d) as f32;
    let image = node.get("image").and_then(Value::as_str).unwrap_or_default();
    let image_url = if crate::images::trusted(image) { crate::images::key(image, 160) } else { String::new() };
    let (min, max) = (num("min", 0.0), num("max", 1.0));
    PluginBlock {
        kind: text("type"),
        id: text("id"),
        text: text("text"),
        sub: text("sub"),
        value: num("value", 0.0).clamp(min.min(max), max.max(min)),
        min,
        max: if max > min { max } else { min + 1.0 },
        input: text("input"),
        on: node.get("on").and_then(Value::as_bool).unwrap_or(false),
        primary: node.get("primary").and_then(Value::as_bool).unwrap_or(false),
        art: crate::state::cached(&image_url),
        image_url: image_url.into(),
    }
}

fn apply(app: &App, op: UiOp) {
    let mut images = Vec::new();
    let mut dirty = false;
    let mut themes = false;
    let mut navigate = None;
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        match op {
            UiOp::Log { plugin, level, text } => log(&s, &plugin, &level, &text),
            UiOp::Notify(text) => crate::menu::toast(app, &text),
            UiOp::Player { action, value } => {
                let player = app.global::<Player>();
                match action.as_str() {
                    "toggle" => player.invoke_toggle(),
                    "play" if !player.get_playing() => player.invoke_toggle(),
                    "pause" if player.get_playing() => player.invoke_toggle(),
                    "next" => player.invoke_next(),
                    "prev" => player.invoke_prev(),
                    "seek" => player.invoke_seek(value.max(0.0) as f32),
                    "volume" => {
                        player.invoke_set_volume(value as f32);
                        player.invoke_commit_volume(value as f32);
                    }
                    "shuffle" => player.invoke_toggle_shuffle(),
                    "repeat" => player.invoke_cycle_repeat(),
                    "like" => player.invoke_save_pressed(),
                    _ => {}
                }
            }
            UiOp::Control { plugin, place, id, label, icon, active, remove } => {
                let model = if place == "topbar" { s.topbar.clone() } else { s.playbar.clone() };
                if remove {
                    retain(&model, |c| !(c.plugin == plugin && c.id == id));
                } else if !upsert(&model, control(&plugin, &id, &label, &icon, active), 4) {
                    log(&s, &plugin, "warn", "a plugin can add at most 4 buttons in each place");
                }
            }
            UiOp::Menu { plugin, id, label, types, remove } => {
                s.menus.retain(|m| !(m.plugin == plugin && m.id == id));
                if !remove {
                    if s.menus.iter().filter(|m| m.plugin == plugin).count() >= 8 {
                        log(&s, &plugin, "warn", "a plugin can add at most 8 menu items");
                    } else {
                        s.menus.push(Menu { plugin, id, label, types });
                    }
                }
            }
            UiOp::Page { plugin, id, title, icon, handle, remove } => {
                let key = (plugin.clone(), id.clone());
                retain(&s.page_links, |p| !(p.plugin == plugin && p.id == id));
                if remove {
                    s.pages.remove(&key);
                    s.blocks.remove(&key);
                } else if s.pages.keys().filter(|(p, _)| *p == plugin).count() >= 4 && !s.pages.contains_key(&key) {
                    log(&s, &plugin, "warn", "a plugin can add at most 4 pages");
                } else {
                    s.page_links.push(PluginPageLink { plugin: plugin.as_str().into(), id: id.as_str().into(), title: title.as_str().into(), icon: icon.as_str().into() });
                    s.pages.insert(key, (title, icon, handle));
                }
            }
            UiOp::Render { plugin, id, nodes } => {
                let key = (plugin, id);
                if s.pages.contains_key(&key) {
                    let rows: Vec<PluginBlock> = nodes.iter().map(block).collect();
                    images.extend(rows.iter().filter(|b| !b.image_url.is_empty()).map(|b| b.image_url.to_string()));
                    if s.open_page.as_ref() == Some(&key) {
                        s.block_model.set_vec(rows.clone());
                    }
                    s.blocks.insert(key, rows);
                }
            }
            UiOp::Section { plugin, id, title, handle, remove } => {
                let key = (plugin.clone(), id);
                if remove {
                    s.sections.remove(&key);
                } else if s.sections.keys().filter(|(p, _)| *p == plugin).count() >= 3 && !s.sections.contains_key(&key) {
                    log(&s, &plugin, "warn", "a plugin can add at most 3 now-playing sections");
                } else {
                    let blocks = s.sections.get(&key).map(|e| e.2.clone()).unwrap_or_default();
                    s.sections.insert(key, (title, handle, blocks));
                }
                rebuild_sections(&s);
            }
            UiOp::SectionRender { plugin, id, nodes } => {
                let rows: Vec<PluginBlock> = nodes.iter().map(block).collect();
                if let Some(entry) = s.sections.get_mut(&(plugin, id)) {
                    images.extend(rows.iter().filter(|b| !b.image_url.is_empty()).map(|b| b.image_url.to_string()));
                    entry.2 = rows;
                    rebuild_sections(&s);
                }
            }
            UiOp::Hide { plugin, element, hidden } => {
                let set = s.hidden.entry(plugin).or_default();
                if hidden {
                    set.insert(element);
                } else {
                    set.remove(&element);
                }
                dirty = true;
            }
            UiOp::Open { plugin, id } => {
                if s.pages.contains_key(&(plugin.clone(), id.clone())) {
                    navigate = Some(pages::Route::Plugin { plugin, id });
                }
            }
            UiOp::Theme { plugin, colors } => {
                if s.running.contains(&plugin) || s.saved.enabled.contains(&plugin) {
                    s.themes.insert(plugin, colors);
                    themes = true;
                }
            }
            UiOp::ThemeReset(plugin) => {
                s.themes.remove(&plugin);
                themes = true;
            }
            UiOp::Navigate(uri) => {
                let mut parts = uri.splitn(3, ':').skip(1);
                let (kind, id) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
                let card = crate::CardData { kind: kind.into(), id: id.into(), uri: uri.as_str().into(), ..Default::default() };
                drop(s);
                app.global::<Store>().invoke_open_card(card);
            }
            UiOp::Started(plugin) => {
                if s.saved.enabled.contains(&plugin) && s.saved.system {
                    s.running.insert(plugin.clone());
                    s.errors.remove(&plugin);
                    log(&s, &plugin, "info", "started");
                } else {
                    send_unload(&s, &plugin);
                }
                dirty = true;
            }
            UiOp::Failed { plugin, error } => {
                s.running.remove(&plugin);
                clear_plugin(&mut s, &plugin);
                log(&s, &plugin, "error", &error);
                s.errors.insert(plugin, error);
                themes = true;
                dirty = true;
            }
            UiOp::Stopped(plugin) => {
                s.running.remove(&plugin);
                dirty = true;
            }
        }
    });
    if themes {
        reset_theme(app);
        reapply_themes(app);
    }
    if let Some(route) = navigate {
        pages::push(app, route);
    }
    if !images.is_empty()
        && let Some(ctx) = STATE.with(|s| s.borrow().ctx.clone())
    {
        ctx.load_images(images);
    }
    if dirty {
        refresh(app);
    }
}

fn send_unload(s: &State, id: &str) {
    if let Some(tx) = &s.tx {
        let _ = tx.send(Cmd::Unload(id.to_owned()));
    }
}

pub fn show_page(app: &App, plugin: &str, id: &str) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let key = (plugin.to_owned(), id.to_owned());
        let title = s.pages.get(&key).map(|p| p.0.clone()).unwrap_or_else(|| "Plugin".into());
        s.block_model.set_vec(s.blocks.get(&key).cloned().unwrap_or_default());
        s.open_page = Some(key);
        let plugins = app.global::<Plugins>();
        plugins.set_page_plugin(plugin.into());
        plugins.set_page_id(id.into());
        plugins.set_page_title(title.into());
    });
}

pub fn left_page() {
    STATE.with(|s| s.borrow_mut().open_page = None);
}

pub fn click(plugin: &str, id: &str) {
    send(Cmd::Action { plugin: plugin.into(), key: id.into(), json: "null".into() });
}

pub fn page_action(block_id: &str, value: &str) {
    let Some((plugin, handle)) = STATE.with(|s| {
        let s = s.borrow();
        let key = s.open_page.clone()?;
        let handle = s.pages.get(&key)?.2.clone();
        Some((key.0, handle))
    }) else {
        return;
    };
    let value: Value = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
    send(Cmd::Action { plugin, key: handle, json: json!({ "id": block_id, "value": value }).to_string() });
}

pub fn section_action(plugin: &str, section: &str, block_id: &str, value: &str) {
    let Some(handle) = STATE.with(|s| s.borrow().sections.get(&(plugin.to_owned(), section.to_owned())).map(|e| e.1.clone())) else {
        return;
    };
    let value: Value = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
    send(Cmd::Action { plugin: plugin.into(), key: handle, json: json!({ "id": block_id, "value": value }).to_string() });
}

pub fn menu_entries(kind: &str) -> Vec<MenuEntry> {
    STATE.with(|s| {
        s.borrow()
            .menus
            .iter()
            .filter(|m| m.types.iter().any(|t| t == kind))
            .map(|m| MenuEntry {
                id: format!("plugin:{}:{}", m.plugin, m.id).into(),
                label: m.label.as_str().into(),
                icon: "M12 3.5 L14.6 9 L20.5 9.6 L16 13.6 L17.3 19.5 L12 16.5 L6.7 19.5 L8 13.6 L3.5 9.6 L9.4 9 Z".into(),
                separator: false,
                checked: false,
            })
            .collect()
    })
}

pub fn menu_pick(action: &str, kind: &str, uri: &str, title: &str) -> bool {
    let Some(rest) = action.strip_prefix("plugin:") else { return false };
    let Some((plugin, id)) = rest.split_once(':') else { return true };
    send(Cmd::Action {
        plugin: plugin.into(),
        key: id.into(),
        json: json!({ "uri": uri, "type": kind, "title": title }).to_string(),
    });
    true
}

pub fn page_links() -> Vec<(String, String, String)> {
    STATE.with(|s| s.borrow().pages.iter().map(|((p, id), (title, _, _))| (p.clone(), id.clone(), title.clone())).collect())
}

pub fn player_state(state: &crate::models::PlayerState) {
    let track = state.track.as_ref().map(|t| {
        json!({
            "id": t.id, "uri": t.uri, "name": t.name, "album": t.album, "duration": t.duration_ms,
            "image": t.image_large.clone().or(t.image_small.clone()).unwrap_or_default(),
            "artists": t.artists.iter().map(|a| json!({ "id": a.id, "name": a.name })).collect::<Vec<_>>(),
        })
    });
    let snapshot = json!({
        "track": track, "isPlaying": state.is_playing, "position": state.position_ms,
        "duration": state.track.as_ref().map(|t| t.duration_ms).unwrap_or(0),
        "volume": state.volume as f64 / 100.0, "shuffle": state.shuffle, "repeat": state.repeat, "context": state.context,
    })
    .to_string();
    let track_id = state.track.as_ref().map(|t| t.uri.clone()).unwrap_or_default();
    let (running, changed, toggled) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let changed = s.last_track != track_id;
        let toggled = s.last_playing != Some(state.is_playing);
        s.last_track = track_id;
        s.last_playing = Some(state.is_playing);
        s.last_state = Some(snapshot.clone());
        (!s.running.is_empty(), changed, toggled)
    });
    if !running {
        return;
    }
    send(Cmd::Emit("state".into(), snapshot.clone()));
    if changed {
        send(Cmd::Emit("songchange".into(), snapshot.clone()));
    }
    if toggled {
        send(Cmd::Emit("onplaypause".into(), snapshot.clone()));
    }
    if state.is_playing {
        send(Cmd::Emit("onprogress".into(), snapshot));
    }
}

pub fn page_changed(page: &str) {
    let running = STATE.with(|s| !s.borrow().running.is_empty());
    if running {
        send(Cmd::Emit("pagechange".into(), json!({ "page": page }).to_string()));
    }
}

pub fn refresh_list(app: &App) {
    refresh(app);
}

pub fn clear_logs() {
    STATE.with(|s| s.borrow().logs.set_vec(Vec::new()));
}

pub fn shutdown() {
    send(Cmd::Shutdown);
}

pub fn apply_image(key: &str, image: &slint::Image) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        for rows in s.blocks.values_mut() {
            for b in rows.iter_mut().filter(|b| b.image_url == key) {
                b.art = image.clone();
            }
        }
        let mut touched = false;
        for entry in s.sections.values_mut() {
            for b in entry.2.iter_mut().filter(|b| b.image_url == key) {
                b.art = image.clone();
                touched = true;
            }
        }
        if touched {
            rebuild_sections(&s);
        }
        let model = s.block_model.clone();
        for i in 0..model.row_count() {
            if let Some(mut row) = model.row_data(i)
                && row.image_url == key
            {
                row.art = image.clone();
                model.set_row_data(i, row);
            }
        }
    });
}

pub fn open_folder() {
    let path = dir();
    let _ = std::fs::create_dir_all(&path);
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}
