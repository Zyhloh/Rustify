use crate::config::Config;
use crate::{App, Prefs};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, Model};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub open_on_play: bool,
    pub show_friends: bool,
    pub minimize_on_close: bool,
    pub on_close: String,
    pub start_with_windows: bool,
    pub eq: bool,
    pub eq_bands: Vec<f32>,
    pub eq_preset: String,
    pub level: String,
    pub mono: bool,
    pub curve: String,
    pub fade: i32,
    pub quality: String,
    pub normalize: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            open_on_play: true,
            show_friends: true,
            minimize_on_close: false,
            on_close: String::new(),
            start_with_windows: false,
            eq: false,
            eq_bands: vec![0.0; 6],
            eq_preset: "Flat".into(),
            level: "normal".into(),
            mono: false,
            curve: "natural".into(),
            fade: 0,
            quality: "very_high".into(),
            normalize: true,
        }
    }
}

pub const PRESETS: [(&str, [f32; 6]); 22] = [
    ("Flat", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Acoustic", [4.5, 4.0, 1.5, 1.5, 3.0, 2.5]),
    ("Bass Booster", [5.5, 4.0, 1.5, 0.0, 0.0, 0.0]),
    ("Bass Reducer", [-5.5, -4.0, -1.5, 0.0, 0.0, 0.0]),
    ("Classical", [4.5, 3.5, -1.5, -1.5, 3.0, 4.0]),
    ("Dance", [3.5, 6.0, 2.0, 0.0, 4.0, 0.5]),
    ("Deep", [4.5, 2.0, 1.5, -2.0, -4.0, -4.5]),
    ("Electronic", [4.0, 3.5, -2.0, 1.5, 1.0, 4.5]),
    ("Hip-Hop", [5.0, 4.0, -1.0, -1.0, 1.5, 3.0]),
    ("Jazz", [4.0, 2.5, -1.5, -1.5, 1.5, 4.0]),
    ("Latin", [4.5, 2.5, 0.0, 0.0, -1.5, 4.0]),
    ("Loudness", [6.0, 4.0, 0.0, -2.0, -1.0, 1.0]),
    ("Lounge", [-3.0, -1.5, 4.0, 2.5, -1.0, 1.5]),
    ("Piano", [3.0, 2.0, 0.0, 2.5, 3.5, 2.0]),
    ("Pop", [-1.5, -1.0, 3.0, 4.0, 2.0, -1.5]),
    ("R&B", [2.5, 7.0, 2.5, -2.0, 2.5, 3.0]),
    ("Rock", [5.0, 4.0, -0.5, -1.5, 2.0, 4.0]),
    ("Small Speakers", [5.5, 4.0, 3.5, 2.0, 1.0, -0.5]),
    ("Spoken Word", [-3.5, -0.5, 3.0, 4.0, 3.5, 0.0]),
    ("Treble Booster", [0.0, 0.0, 0.0, 1.5, 4.0, 5.5]),
    ("Treble Reducer", [0.0, 0.0, 0.0, -1.5, -4.0, -5.5]),
    ("Vocal Booster", [-1.5, -3.0, 3.0, 4.0, 2.5, -1.0]),
];

pub fn audio_body(app: &App) -> serde_json::Value {
    let prefs = app.global::<Prefs>();
    let bands: Vec<f32> = (0..6).map(|i| prefs.get_eq_bands().row_data(i).unwrap_or(0.0)).collect();
    serde_json::json!({
        "eq": prefs.get_eq_on(),
        "bands": bands,
        "level": prefs.get_audio_level().as_str(),
        "mono": prefs.get_mono(),
        "curve": prefs.get_volume_curve().as_str(),
        "fade": prefs.get_fade().clamp(0, 12),
        "quality": prefs.get_quality().as_str(),
        "normalize": prefs.get_normalize(),
    })
}

pub fn set_band(app: &App, index: usize, value: f32) {
    let prefs = app.global::<Prefs>();
    let bands = prefs.get_eq_bands();
    if index < bands.row_count() {
        bands.set_row_data(index, value.clamp(-12.0, 12.0));
        prefs.set_eq_preset("Custom".into());
    }
}

pub fn apply_preset(app: &App, name: &str) {
    let Some((label, values)) = PRESETS.iter().find(|(n, _)| *n == name) else { return };
    let prefs = app.global::<Prefs>();
    prefs.set_eq_bands(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(values.to_vec()))));
    prefs.set_eq_preset((*label).into());
    prefs.set_eq_on(true);
}

fn path() -> std::path::PathBuf {
    Config::data_dir().join("settings.json")
}

pub fn load(app: &App) {
    let settings: Settings = std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let prefs = app.global::<Prefs>();
    prefs.set_open_on_play(settings.open_on_play);
    prefs.set_show_friends(settings.show_friends);
    let on_close = match settings.on_close.as_str() {
        "exit" | "tray" | "taskbar" => settings.on_close.clone(),
        _ if settings.minimize_on_close => "taskbar".into(),
        _ => "tray".into(),
    };
    prefs.set_on_close(on_close.into());
    let autostart = autostart_enabled();
    if autostart {
        set_autostart(true);
    }
    prefs.set_start_with_windows(autostart);
    let mut bands = settings.eq_bands.clone();
    bands.resize(6, 0.0);
    prefs.set_eq_bands(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(bands))));
    prefs.set_eq_on(settings.eq);
    prefs.set_eq_preset(settings.eq_preset.as_str().into());
    prefs.set_audio_level(settings.level.as_str().into());
    prefs.set_mono(settings.mono);
    prefs.set_volume_curve(settings.curve.as_str().into());
    prefs.set_fade(settings.fade.clamp(0, 12));
    prefs.set_quality(settings.quality.as_str().into());
    prefs.set_normalize(settings.normalize);
}

pub fn save(app: &App) {
    let prefs = app.global::<Prefs>();
    let settings = Settings {
        open_on_play: prefs.get_open_on_play(),
        show_friends: prefs.get_show_friends(),
        minimize_on_close: false,
        on_close: prefs.get_on_close().to_string(),
        start_with_windows: prefs.get_start_with_windows(),
        eq: prefs.get_eq_on(),
        eq_bands: (0..6).map(|i| prefs.get_eq_bands().row_data(i).unwrap_or(0.0)).collect(),
        eq_preset: prefs.get_eq_preset().to_string(),
        level: prefs.get_audio_level().to_string(),
        mono: prefs.get_mono(),
        curve: prefs.get_volume_curve().to_string(),
        fade: prefs.get_fade(),
        quality: prefs.get_quality().to_string(),
        normalize: prefs.get_normalize(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&settings) {
        let _ = std::fs::create_dir_all(Config::data_dir());
        let _ = std::fs::write(path(), json);
    }
    set_autostart(settings.start_with_windows);
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
fn autostart_enabled() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>("Rustify"))
        .is_ok()
}

#[cfg(windows)]
fn set_autostart(enabled: bool) {
    use winreg::{RegKey, enums::{HKEY_CURRENT_USER, KEY_WRITE}};
    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_WRITE) else {
        return;
    };
    if enabled {
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value("Rustify", &format!("\"{}\" --tray", exe.display()));
        }
    } else {
        let _ = key.delete_value("Rustify");
    }
}

#[cfg(not(windows))]
fn autostart_enabled() -> bool {
    false
}

#[cfg(not(windows))]
fn set_autostart(_enabled: bool) {}

pub fn cache_size() -> u64 {
    fn walk(dir: &std::path::Path) -> u64 {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| match entry.metadata() {
                Ok(m) if m.is_dir() => walk(&entry.path()),
                Ok(m) => m.len(),
                Err(_) => 0,
            })
            .sum()
    }
    walk(&Config::data_dir().join("images")) + walk(&Config::data_dir().join("webview"))
}

pub fn prune_image_cache(limit: u64) {
    let dir = Config::data_dir().join("images");
    let mut files: Vec<(std::time::SystemTime, u64, std::path::PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            meta.is_file().then(|| (meta.modified().unwrap_or(std::time::UNIX_EPOCH), meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= limit {
        return;
    }
    files.sort_by_key(|(at, _, _)| *at);
    for (_, len, path) in files {
        if total <= limit * 8 / 10 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

pub fn clear_image_cache() {
    let dir = Config::data_dir().join("images");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", bytes as f64 / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        _ => format!("{:.2} GB", bytes as f64 / 1_073_741_824.0),
    }
}
