use super::{
    Found, STATE, data_dir, deactivate, dir, install as installer, manifest::{self, Kind}, refresh as refresh_all, reapply_themes, reset_theme,
    review::Level, save, staging_dir, sync, system_on,
};
use crate::{App, PluginFinding, PluginInfo, Plugins, pages};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};
use std::{path::Path, rc::Rc, time::Duration};

pub fn permissions_text(perms: &[String]) -> String {
    let names: Vec<&str> = perms
        .iter()
        .map(|p| match p.as_str() {
            "player" => "control playback",
            "spotify" => "read your Spotify data",
            "theme" => "change Rustify's colors",
            "network" => "connect to the websites it lists",
            "audio" => "play audio from the websites it lists",
            "playlists" => "add songs to your playlists",
            _ => "unknown",
        })
        .collect();
    if names.is_empty() { "No special permissions".into() } else { capitalize(&names.join(", ")) }
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

pub fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

fn access(m: &manifest::Manifest) -> Vec<SharedString> {
    let mut out: Vec<SharedString> = Vec::new();
    if m.kind == Kind::Theme {
        out.push("Change Rustify's colors. Themes can't run code.".into());
        return out;
    }
    out.push("See what's playing and add buttons, menu items and pages".into());
    for p in &m.permissions {
        out.push(
            match p.as_str() {
                "player" => "Control playback: play, pause, skip, seek, volume, shuffle and repeat",
                "spotify" => "Read your Spotify account data through the Web API, such as playlists and top tracks. It can't change anything",
                "theme" => "Change Rustify's colors",
                "network" => "Send and receive data from the websites listed below",
                "audio" => "Play audio from the websites listed below through Rustify's player",
                "playlists" => "Add its own songs to your Spotify playlists. They show up as local files, like in Spotify's app",
                _ => continue,
            }
            .into(),
        );
    }
    if m.network.is_empty() {
        out.push("No access to your files, other apps or the internet".into());
    } else {
        out.push(format!("Websites: {}", m.network.join(", ")).into());
        out.push("No access to your files, other apps or your local network".into());
    }
    out
}

pub fn load_icon(path: &Path) -> Option<slint::Image> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > manifest::MAX_ICON {
        return None;
    }
    let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.thumbnail(160, 160).to_rgba8();
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height());
    Some(slint::Image::from_rgba8(buffer))
}

const TINTS: [(u8, u8, u8); 8] = [(0x8d, 0x67, 0xab), (0x27, 0x85, 0x6a), (0x1e, 0x32, 0x64), (0xe8, 0x11, 0x5b), (0xba, 0x5d, 0x07), (0x14, 0x8a, 0x08), (0x50, 0x9b, 0xf5), (0xaf, 0x28, 0x96)];

fn tint(id: &str) -> Color {
    let hash = id.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let (r, g, b) = TINTS[hash as usize % TINTS.len()];
    Color::from_rgb_u8(r, g, b)
}

fn letter(name: &str) -> SharedString {
    name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default().into()
}

fn info(f: &Found, s: &super::State) -> PluginInfo {
    let Some(m) = &f.manifest else {
        return PluginInfo {
            id: f.folder.as_str().into(),
            kind: "plugin".into(),
            name: f.folder.as_str().into(),
            error: f.error.as_str().into(),
            letter: letter(&f.folder),
            tint: tint(&f.folder),
            folder: f.folder.as_str().into(),
            risk: "broken".into(),
            risk_title: "Can't be loaded".into(),
            ..Default::default()
        };
    };
    let swatch = |key: &str| {
        m.theme.get(key).and_then(|v| super::parse_color(v)).or_else(|| s.defaults.get(key).copied()).unwrap_or_default()
    };
    let level = f.review.level();
    let findings: Vec<PluginFinding> =
        f.review.findings.iter().map(|x| PluginFinding { level: x.level.as_str().into(), text: x.text.as_str().into() }).collect();
    PluginInfo {
        id: m.id.as_str().into(),
        kind: m.kind.as_str().into(),
        name: m.name.as_str().into(),
        version: m.version.as_str().into(),
        author: m.author.as_str().into(),
        author_url: m.author_url.as_str().into(),
        repository: m.repository.as_str().into(),
        repository_label: manifest::link_label(&m.repository).into(),
        homepage: m.homepage.as_str().into(),
        homepage_label: manifest::link_label(&m.homepage).into(),
        description: m.description.as_str().into(),
        permissions: permissions_text(&m.permissions).into(),
        access: ModelRc::from(Rc::new(VecModel::from(access(m)))),
        error: s.errors.get(&m.id).cloned().unwrap_or_default().into(),
        enabled: s.saved.enabled.contains(&m.id),
        running: s.running.contains(&m.id),
        valid: true,
        risk: level.map(|l| l.as_str()).unwrap_or("safe").into(),
        risk_title: match level {
            Some(Level::Caution) => "Potentially harmful",
            Some(_) => "Review before turning on",
            None => "No issues found",
        }
        .into(),
        findings: ModelRc::from(Rc::new(VecModel::from(findings))),
        endpoints: f.review.endpoints.join("\n").into(),
        has_icon: f.icon.is_some(),
        icon: f.icon.clone().unwrap_or_default(),
        letter: letter(&m.name),
        tint: tint(&m.id),
        swatch_base: swatch("base"),
        swatch_surface: swatch("surface"),
        swatch_raised: swatch("surface-raised"),
        swatch_accent: swatch("accent"),
        swatch_text: swatch("text"),
        swatch_dim: swatch("text-dim"),
        folder: f.folder.as_str().into(),
    }
}

fn kind_of(f: &Found) -> Kind {
    f.manifest.as_ref().map(|m| m.kind).unwrap_or(Kind::Plugin)
}

fn needs_attention(f: &Found, s: &super::State) -> bool {
    match &f.manifest {
        None => true,
        Some(m) => s.errors.contains_key(&m.id) || f.review.level().is_some(),
    }
}

fn matches(f: &Found, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    match &f.manifest {
        Some(m) => [&m.name, &m.author, &m.description, &m.id].iter().any(|t| t.to_lowercase().contains(&q)),
        None => f.folder.to_lowercase().contains(&q),
    }
}

pub fn refresh(app: &App) {
    let plugins = app.global::<Plugins>();
    STATE.with(|s| {
        let s = s.borrow();
        let tab = if s.tab == "themes" { Kind::Theme } else { Kind::Plugin };
        let rows: Vec<PluginInfo> = s
            .found
            .iter()
            .filter(|f| kind_of(f) == tab)
            .filter(|f| matches(f, &s.query))
            .filter(|f| {
                let id = f.manifest.as_ref().map(|m| m.id.as_str()).unwrap_or_default();
                match s.filter.as_str() {
                    "on" => s.saved.enabled.contains(id),
                    "off" => !s.saved.enabled.contains(id),
                    "attention" => needs_attention(f, &s),
                    _ => true,
                }
            })
            .map(|f| info(f, &s))
            .collect();
        s.shown.set_vec(rows);
        let count = |k: Kind| s.found.iter().filter(|f| kind_of(f) == k).count() as i32;
        plugins.set_plugin_count(count(Kind::Plugin));
        plugins.set_theme_count(count(Kind::Theme));
        plugins.set_attention_count(s.found.iter().filter(|f| kind_of(f) == tab && needs_attention(f, &s)).count() as i32);
        plugins.set_running_count(s.running.len() as i32);
        let active = s
            .found
            .iter()
            .filter_map(|f| f.manifest.as_ref())
            .find(|m| m.kind == Kind::Theme && s.saved.enabled.contains(&m.id))
            .map(|m| m.id.clone())
            .unwrap_or_default();
        plugins.set_active_theme(active.into());
        plugins.set_tab(s.tab.as_str().into());
        plugins.set_filter(s.filter.as_str().into());
        let selected = s.selected.as_ref().and_then(|id| s.found.iter().find(|f| f.manifest.as_ref().is_some_and(|m| &m.id == id) || &f.folder == id));
        plugins.set_has_selected(selected.is_some());
        if let Some(f) = selected {
            plugins.set_selected(info(f, &s));
        }
    });
}

pub fn set_query(app: &App, query: &str) {
    STATE.with(|s| s.borrow_mut().query = query.chars().take(80).collect());
    refresh_all(app);
}

pub fn set_tab(app: &App, tab: &str) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.tab = if tab == "themes" { "themes".into() } else { "plugins".into() };
        s.filter = "all".into();
    });
    refresh_all(app);
}

pub fn set_filter(app: &App, filter: &str) {
    STATE.with(|s| {
        s.borrow_mut().filter = match filter {
            "on" | "off" | "attention" => filter.into(),
            _ => "all".into(),
        }
    });
    refresh_all(app);
}

pub fn select(app: &App, id: &str) {
    STATE.with(|s| s.borrow_mut().selected = Some(id.to_owned()));
    refresh_all(app);
}

pub fn use_default_theme(app: &App) {
    let active: Vec<String> = STATE.with(|s| {
        let s = s.borrow();
        s.found
            .iter()
            .filter_map(|f| f.manifest.as_ref())
            .filter(|m| m.kind == Kind::Theme && s.saved.enabled.contains(&m.id))
            .map(|m| m.id.clone())
            .collect()
    });
    for id in &active {
        STATE.with(|s| s.borrow_mut().saved.enabled.remove(id));
        deactivate(id);
    }
    save();
    reset_theme(app);
    reapply_themes(app);
    refresh_all(app);
}

pub fn open_link(url: &str) {
    if manifest::valid_link(url) {
        crate::panels::open_url(url);
    }
}

fn folder_of(id: &str) -> Option<std::path::PathBuf> {
    let folder = STATE.with(|s| s.borrow().found.iter().find(|f| f.manifest.as_ref().is_some_and(|m| m.id == id) || f.folder == id).map(|f| f.folder.clone()))?;
    let root = dir();
    let path = root.join(&folder);
    let canonical = path.canonicalize().ok()?;
    (canonical.parent()? == root.canonicalize().ok()? && canonical.is_dir()).then_some(path)
}

pub fn open_plugin_folder(id: &str) {
    if let Some(path) = folder_of(id) {
        #[cfg(windows)]
        let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
    }
}

pub fn uninstall(app: &App, id: &str) {
    let Some(path) = folder_of(id) else { return };
    let name = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.saved.enabled.remove(id);
        s.selected = None;
        s.found.iter().find_map(|f| f.manifest.as_ref().filter(|m| m.id == id).map(|m| m.name.clone())).unwrap_or_else(|| id.to_owned())
    });
    save();
    deactivate(id);
    reset_theme(app);
    reapply_themes(app);
    match std::fs::remove_dir_all(&path) {
        Ok(()) => {
            if manifest::valid_id(id) {
                let _ = std::fs::remove_file(data_dir().join(format!("{id}.json")));
            }
            crate::menu::toast(app, &format!("Removed {name}"));
        }
        Err(_) => crate::menu::toast(app, &format!("Couldn't remove {name}. Close any program using its files and try again")),
    }
    STATE.with(|s| s.borrow_mut().found.retain(|f| f.folder != path.file_name().and_then(|n| n.to_str()).unwrap_or_default()));
    if matches!(pages::current(), pages::Route::Extension { .. }) {
        pages::push(app, pages::Route::Extensions);
    }
    refresh_all(app);
}

fn busy(app: &App) -> bool {
    let plugins = app.global::<Plugins>();
    if plugins.get_installing() {
        return true;
    }
    plugins.set_installing(true);
    plugins.set_install_error(SharedString::new());
    false
}

fn finish(app: &App, result: Option<Result<installer::Installed, String>>) {
    let plugins = app.global::<Plugins>();
    plugins.set_installing(false);
    match result {
        None => {}
        Some(Ok(done)) => {
            plugins.set_link_open(false);
            sync(app);
            let what = if done.kind == Kind::Theme { "theme" } else { "plugin" };
            let verb = if done.replaced { "Updated" } else { "Installed" };
            crate::menu::toast(app, &format!("{verb} {what} {}", done.name));
            STATE.with(|s| s.borrow_mut().tab = if done.kind == Kind::Theme { "themes".into() } else { "plugins".into() });
            pages::push(app, pages::Route::Extension { id: done.id });
        }
        Some(Err(error)) => {
            plugins.set_install_error(format!("Couldn't install: {error}").into());
            crate::menu::toast(app, &format!("Couldn't install: {error}"));
        }
    }
}

pub fn install_file(app: &App) {
    if !system_on() || busy(app) {
        return;
    }
    let weak = app.as_weak();
    let (root, staging) = (dir(), staging_dir());
    let _ = std::thread::Builder::new().name("rustify-plugin-install".into()).spawn(move || {
        let picked = rfd::FileDialog::new().set_title("Install a plugin or theme").add_filter("Plugin or theme", &["zip"]).pick_file();
        let result = picked.map(|p| installer::from_path(&p, &root, &staging));
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                finish(&app, result);
            }
        });
    });
}

pub fn install_link(app: &App, url: &str) {
    if !system_on() {
        return;
    }
    let Some(source) = installer::github_source(url) else {
        app.global::<Plugins>().set_install_error("Paste a link to a GitHub repository, like https://github.com/name/plugin".into());
        return;
    };
    let Some(ctx) = STATE.with(|s| s.borrow().ctx.clone()) else { return };
    if busy(app) {
        return;
    }
    let weak = app.as_weak();
    let (root, staging) = (dir(), staging_dir());
    ctx.rt.spawn(async move {
        let result = download(&source.zip_url).await;
        let result = match result {
            Ok(bytes) => tokio::task::spawn_blocking(move || installer::from_zip(&bytes, &source.sub, &root, &staging))
                .await
                .unwrap_or_else(|_| Err("the install stopped unexpectedly".into())),
            Err(e) => Err(e),
        };
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                finish(&app, Some(result));
            }
        });
    });
}

async fn download(url: &str) -> Result<Vec<u8>, String> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let trusted = attempt.url().scheme() == "https"
            && attempt.url().host_str().is_some_and(|h| h == "github.com" || h.ends_with(".github.com") || h == "codeload.github.com");
        if trusted && attempt.previous().len() < 3 { attempt.follow() } else { attempt.stop() }
    });
    let client = reqwest::Client::builder()
        .redirect(policy)
        .timeout(Duration::from_secs(30))
        .user_agent("Rustify")
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client.get(url).send().await.map_err(|_| "couldn't reach GitHub".to_string())?;
    match response.status().as_u16() {
        200 => {}
        404 => return Err("that repository or branch wasn't found. Is it public?".into()),
        code => return Err(format!("GitHub answered with error {code}")),
    }
    if response.content_length().is_some_and(|l| l as usize > installer::MAX_ARCHIVE) {
        return Err("the download is larger than 5 MB".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "the download was interrupted".to_string())? {
        if bytes.len() + chunk.len() > installer::MAX_ARCHIVE {
            return Err("the download is larger than 5 MB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub fn cancel_install_link(app: &App) {
    let plugins = app.global::<Plugins>();
    plugins.set_link_open(false);
    plugins.set_install_error(SharedString::new());
}

pub fn hover_drop(app: &App, hovering: bool) {
    app.global::<Plugins>().set_dropping(hovering && system_on());
}

pub fn dropped(app: &App, path: std::path::PathBuf) {
    app.global::<Plugins>().set_dropping(false);
    if !system_on() {
        crate::menu::toast(app, "Turn on Plugins & themes in Settings to install add-ons");
        return;
    }
    if busy(app) {
        return;
    }
    let weak = app.as_weak();
    let (root, staging) = (dir(), staging_dir());
    let _ = std::thread::Builder::new().name("rustify-plugin-install".into()).spawn(move || {
        let result = installer::from_path(&path, &root, &staging);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                finish(&app, Some(result));
            }
        });
    });
}
