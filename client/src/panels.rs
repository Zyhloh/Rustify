use crate::models::TrackRow;
use crate::pages::track_data;
use crate::{App, DeviceData, FriendData, LyricLine, Panels, TrackData, images, state};
use serde::Deserialize;
use slint::{Color, ComponentHandle, Image, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

#[derive(Debug, Clone, Deserialize)]
pub struct Queue {
    pub now: Option<TrackRow>,
    pub next: Vec<TrackRow>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub current: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Friend {
    pub name: String,
    pub image: Option<String>,
    pub track_uri: String,
    pub track: String,
    pub artist: String,
    pub context: String,
    pub context_uri: String,
    pub minutes_ago: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Line {
    pub start_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Lyrics {
    pub synced: bool,
    pub lines: Vec<Line>,
    pub background: String,
    pub provider: String,
}

struct PanelState {
    queue: Rc<VecModel<TrackData>>,
    queue_uris: RefCell<Vec<String>>,
    recent: Rc<VecModel<TrackData>>,
    recent_uris: RefCell<Vec<String>>,
    devices: Rc<VecModel<DeviceData>>,
    friends: Rc<VecModel<FriendData>>,
    lyrics: Rc<VecModel<LyricLine>>,
    lyrics_track: RefCell<String>,
}

thread_local! {
    static PANELS: PanelState = PanelState {
        queue: Rc::new(VecModel::default()),
        queue_uris: RefCell::new(Vec::new()),
        recent: Rc::new(VecModel::default()),
        recent_uris: RefCell::new(Vec::new()),
        devices: Rc::new(VecModel::default()),
        friends: Rc::new(VecModel::default()),
        lyrics: Rc::new(VecModel::default()),
        lyrics_track: RefCell::new(String::new()),
    };
}

pub fn install(app: &App) {
    let panels = app.global::<Panels>();
    PANELS.with(|p| {
        panels.set_queue(ModelRc::from(p.queue.clone()));
        panels.set_recent(ModelRc::from(p.recent.clone()));
        panels.set_devices(ModelRc::from(p.devices.clone()));
        panels.set_friends(ModelRc::from(p.friends.clone()));
        panels.set_lyrics(ModelRc::from(p.lyrics.clone()));
    });
}

fn rows(list: &[TrackRow], keys: &mut Vec<String>) -> Vec<TrackData> {
    list.iter()
        .enumerate()
        .map(|(i, t)| {
            let row = track_data(i, t, true, false);
            if !row.image_url.is_empty() {
                keys.push(row.image_url.to_string());
            }
            row
        })
        .collect()
}

pub fn set_queue(app: &App, queue: &Queue) -> Vec<String> {
    let panels = app.global::<Panels>();
    let mut keys = Vec::new();
    match &queue.now {
        Some(now) => {
            let row = track_data(0, now, true, false);
            if !row.image_url.is_empty() {
                keys.push(row.image_url.to_string());
            }
            panels.set_queue_now(row);
            panels.set_has_queue_now(true);
        }
        None => panels.set_has_queue_now(false),
    }
    let next = rows(&queue.next, &mut keys);
    PANELS.with(|p| {
        *p.queue_uris.borrow_mut() = queue.next.iter().map(|t| t.uri.clone()).collect();
        p.queue.set_vec(next);
    });
    panels.set_queue_loading(false);
    keys
}

pub fn set_recent(app: &App, list: &[TrackRow]) -> Vec<String> {
    let mut keys = Vec::new();
    let data = rows(list, &mut keys);
    PANELS.with(|p| {
        *p.recent_uris.borrow_mut() = list.iter().map(|t| t.uri.clone()).collect();
        p.recent.set_vec(data);
    });
    app.global::<Panels>().set_queue_loading(false);
    keys
}

pub fn queue_from(index: usize) -> Vec<String> {
    PANELS.with(|p| p.queue_uris.borrow().iter().skip(index).cloned().collect())
}

pub fn recent_from(index: usize) -> Vec<String> {
    PANELS.with(|p| p.recent_uris.borrow().iter().skip(index).cloned().collect())
}

pub fn set_devices(app: &App, list: &[Device]) {
    let data: Vec<DeviceData> = list
        .iter()
        .map(|d| DeviceData {
            id: d.id.as_str().into(),
            name: d.name.as_str().into(),
            kind: d.kind.as_str().into(),
            active: d.active,
            current: d.current,
        })
        .collect();
    PANELS.with(|p| p.devices.set_vec(data));
    app.global::<Panels>().set_devices_loading(false);
}

fn ago(minutes: u64) -> String {
    match minutes {
        0..=1 => "now".into(),
        2..=59 => format!("{minutes} min"),
        60..=1439 => format!("{} hr", minutes / 60),
        _ => format!("{} d", minutes / 1440),
    }
}

pub fn set_friends(app: &App, list: &[Friend]) -> Vec<String> {
    let mut keys = Vec::new();
    let data: Vec<FriendData> = list
        .iter()
        .map(|f| {
            let key = f.image.as_deref().map(|u| images::key(u, 80)).unwrap_or_default();
            if !key.is_empty() {
                keys.push(key.clone());
            }
            FriendData {
                name: f.name.as_str().into(),
                art: state::cached(&key),
                image_url: key.into(),
                track: f.track.as_str().into(),
                track_uri: f.track_uri.as_str().into(),
                artist: f.artist.as_str().into(),
                context: f.context.as_str().into(),
                context_uri: f.context_uri.as_str().into(),
                ago: ago(f.minutes_ago).into(),
            }
        })
        .collect();
    PANELS.with(|p| p.friends.set_vec(data));
    app.global::<Panels>().set_friends_loading(false);
    keys
}

fn parse_color(hex: &str) -> Color {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x3a3a3a);
    Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn lyrics_track() -> String {
    PANELS.with(|p| p.lyrics_track.borrow().clone())
}

pub fn begin_lyrics(app: &App, track_id: &str) {
    let panels = app.global::<Panels>();
    PANELS.with(|p| {
        *p.lyrics_track.borrow_mut() = track_id.to_owned();
        p.lyrics.set_vec(Vec::new());
    });
    panels.set_lyrics_state("loading".into());
    panels.set_lyrics_provider("".into());
}

pub fn set_lyrics(app: &App, track_id: &str, lyrics: Option<Lyrics>) {
    if lyrics_track() != track_id {
        return;
    }
    let panels = app.global::<Panels>();
    let Some(lyrics) = lyrics.filter(|l| !l.lines.is_empty()) else {
        panels.set_lyrics_state("none".into());
        panels.set_lyrics_color(Color::from_rgb_u8(0x3a, 0x3a, 0x3a));
        return;
    };
    let count = lyrics.lines.len();
    let data: Vec<LyricLine> = lyrics
        .lines
        .iter()
        .enumerate()
        .map(|(i, line)| LyricLine {
            start: line.start_ms as f32,
            end: lyrics.lines.get(i + 1).map(|n| n.start_ms as f32).unwrap_or(f32::MAX),
            text: line.text.as_str().into(),
        })
        .collect();
    PANELS.with(|p| p.lyrics.set_vec(data));
    panels.set_lyrics_synced(lyrics.synced && count > 1);
    panels.set_lyrics_color(parse_color(&lyrics.background));
    panels.set_lyrics_provider(lyrics.provider.as_str().into());
    panels.set_lyrics_state("ready".into());
}

fn patch(model: &VecModel<TrackData>, key: &str, image: &Image) {
    for i in 0..model.row_count() {
        if let Some(mut row) = model.row_data(i)
            && row.image_url == key {
                row.art = image.clone();
                model.set_row_data(i, row);
            }
    }
}

pub fn apply_image(app: &App, key: &str, image: &Image) {
    let panels = app.global::<Panels>();
    let mut now = panels.get_queue_now();
    if now.image_url == key {
        now.art = image.clone();
        panels.set_queue_now(now);
    }
    PANELS.with(|p| {
        patch(&p.queue, key, image);
        patch(&p.recent, key, image);
        for i in 0..p.friends.row_count() {
            if let Some(mut row) = p.friends.row_data(i)
                && row.image_url == key {
                    row.art = image.clone();
                    p.friends.set_row_data(i, row);
                }
        }
    });
}

pub fn safe_url(url: &str) -> bool {
    url.starts_with("https://")
        && url.len() < 300
        && url.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=%".contains(c))
        && !url.contains('"')
}

pub fn open_url(url: &str) {
    if !safe_url(url) {
        tracing::warn!("refused to open url");
        return;
    }
    #[cfg(windows)]
    let _ = std::process::Command::new("rundll32.exe").args(["url.dll,FileProtocolHandler", url]).spawn();
}
