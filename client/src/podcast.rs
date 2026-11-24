use crate::models::{Episode, Show};
use crate::{App, EpisodeData, ShowData, Store, images, state};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

const ART_PX: u32 = 160;
const HEADER_PX: u32 = 480;
pub const YOUR_EPISODES: &str = "episodes";

struct PodcastState {
    episodes: Rc<VecModel<EpisodeData>>,
    uris: RefCell<Vec<String>>,
}

thread_local! {
    static PODCAST: PodcastState = PodcastState {
        episodes: Rc::new(VecModel::default()),
        uris: RefCell::new(Vec::new()),
    };
}

pub fn install(app: &App) {
    PODCAST.with(|p| app.global::<Store>().set_show_episodes(ModelRc::from(p.episodes.clone())));
}

pub fn begin(app: &App, id: &str) {
    let store = app.global::<Store>();
    let mine = id == YOUR_EPISODES;
    store.set_show(ShowData {
        id: id.into(),
        kind: if mine { "episodes".into() } else { "show".into() },
        title: if mine { "Your Episodes".into() } else { "".into() },
        ..Default::default()
    });
    store.set_show_color(if mine { Color::from_rgb_u8(0x00, 0x64, 0x50) } else { Color::from_rgb_u8(0x3a, 0x3a, 0x3a) });
    store.set_show_loading(true);
    PODCAST.with(|p| {
        p.episodes.set_vec(Vec::new());
        p.uris.borrow_mut().clear();
    });
}

fn episode_data(e: &Episode, keys: &mut Vec<String>) -> EpisodeData {
    let key = e.image.as_deref().map(|u| images::key(u, ART_PX)).unwrap_or_default();
    if !key.is_empty() {
        keys.push(key.clone());
    }
    EpisodeData {
        id: e.id.as_str().into(),
        uri: e.uri.as_str().into(),
        title: e.name.as_str().into(),
        description: e.description.split_whitespace().collect::<Vec<_>>().join(" ").into(),
        date: pretty_date(&e.date).into(),
        duration: pretty_duration(&e.duration).into(),
        explicit: e.explicit,
        show: e.show.as_str().into(),
        show_id: e.show_id.as_str().into(),
        art: state::cached(&key),
        image_url: key.into(),
    }
}

pub fn set_show(app: &App, show: &Show) -> Vec<String> {
    let store = app.global::<Store>();
    if store.get_show().id != show.id.as_str() {
        return Vec::new();
    }
    let mut keys = Vec::new();
    let key = show.image.as_deref().map(|u| images::key(u, HEADER_PX)).unwrap_or_default();
    if !key.is_empty() {
        keys.push(key.clone());
    }
    let mine = show.id == YOUR_EPISODES;
    let count = show.episodes.len();
    store.set_show(ShowData {
        id: show.id.as_str().into(),
        uri: show.uri.as_str().into(),
        kind: if mine { "episodes".into() } else { "show".into() },
        title: show.title.as_str().into(),
        publisher: show.publisher.as_str().into(),
        description: show.description.trim().into(),
        meta: if mine { format!("{count} episode{}", if count == 1 { "" } else { "s" }).into() } else { "".into() },
        saved: show.saved,
        art: state::cached(&key),
        image_url: key.into(),
    });
    let rows: Vec<EpisodeData> = show.episodes.iter().map(|e| episode_data(e, &mut keys)).collect();
    PODCAST.with(|p| {
        *p.uris.borrow_mut() = show.episodes.iter().map(|e| e.uri.clone()).collect();
        p.episodes.set_vec(rows);
    });
    store.set_show_loading(false);
    keys
}

pub fn uris() -> Vec<String> {
    PODCAST.with(|p| p.uris.borrow().clone())
}

pub fn set_saved(app: &App, saved: bool) {
    let store = app.global::<Store>();
    let mut show = store.get_show();
    show.saved = saved;
    store.set_show(show);
}

pub fn apply_image(app: &App, key: &str, image: &Image, accent: [f32; 3]) {
    let store = app.global::<Store>();
    let mut show = store.get_show();
    if show.image_url == key {
        show.art = image.clone();
        store.set_show(show);
        store.set_show_color(state::shade(accent, 0.46));
    }
    PODCAST.with(|p| {
        for i in 0..p.episodes.row_count() {
            if let Some(mut row) = p.episodes.row_data(i)
                && row.image_url == key {
                    row.art = image.clone();
                    p.episodes.set_row_data(i, row);
                }
        }
    });
}

fn pretty_duration(raw: &str) -> String {
    let total = raw.split(':').filter_map(|p| p.parse::<u64>().ok()).fold(0, |acc, p| acc * 60 + p);
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    match (h, m) {
        (0, 0) => format!("{s} sec"),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} hr"),
        (h, m) => format!("{h} hr {m} min"),
    }
}

fn pretty_date(raw: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = raw.split('-');
    let year = parts.next().unwrap_or_default();
    let month = parts.next().and_then(|m| m.parse::<usize>().ok());
    let day = parts.next().and_then(|d| d.parse::<u32>().ok());
    match (month, day) {
        (Some(m @ 1..=12), Some(d)) => format!("{} {d}, {year}", MONTHS[m - 1]),
        (Some(m @ 1..=12), None) => format!("{} {year}", MONTHS[m - 1]),
        _ => year.to_owned(),
    }
}
