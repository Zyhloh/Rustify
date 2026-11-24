use crate::config::Config;
use crate::models::{Card, Section, TrackRow};
use crate::pages::{card_data, track_data};
use crate::{App, CardData, SectionData, Store, TileData, TrackData, images, state};
use serde::{Deserialize, Serialize};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

const RECENT_LIMIT: usize = 12;

#[derive(Debug, Clone, Deserialize)]
pub struct Results {
    pub query: String,
    pub filter: String,
    pub top: Option<Card>,
    pub tracks: Vec<TrackRow>,
    pub sections: Vec<Section>,
    pub grid: Vec<Card>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tile {
    pub key: String,
    pub title: String,
    pub color: String,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Category {
    pub title: String,
    pub color: String,
    pub tracks: Vec<TrackRow>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Recent {
    id: String,
    uri: String,
    kind: String,
    title: String,
    subtitle: String,
    image: Option<String>,
}

struct SearchState {
    tracks: Rc<VecModel<TrackData>>,
    grid: Rc<VecModel<CardData>>,
    sections: Rc<VecModel<SectionData>>,
    rows: RefCell<Vec<Rc<VecModel<CardData>>>>,
    tiles: Rc<VecModel<TileData>>,
    recent: Rc<VecModel<CardData>>,
    recent_raw: RefCell<Vec<Recent>>,
    category_tracks: Rc<VecModel<TrackData>>,
    category_sections: Rc<VecModel<SectionData>>,
    category_rows: RefCell<Vec<Rc<VecModel<CardData>>>>,
    track_uris: RefCell<Vec<String>>,
    category_uris: RefCell<Vec<String>>,
}

thread_local! {
    static SEARCH: SearchState = SearchState {
        tracks: Rc::new(VecModel::default()),
        grid: Rc::new(VecModel::default()),
        sections: Rc::new(VecModel::default()),
        rows: RefCell::new(Vec::new()),
        tiles: Rc::new(VecModel::default()),
        recent: Rc::new(VecModel::default()),
        recent_raw: RefCell::new(Vec::new()),
        category_tracks: Rc::new(VecModel::default()),
        category_sections: Rc::new(VecModel::default()),
        category_rows: RefCell::new(Vec::new()),
        track_uris: RefCell::new(Vec::new()),
        category_uris: RefCell::new(Vec::new()),
    };
}

fn recent_path() -> std::path::PathBuf {
    Config::data_dir().join("recent_searches.json")
}

fn to_card(r: &Recent) -> Card {
    Card {
        id: r.id.clone(),
        uri: r.uri.clone(),
        kind: r.kind.clone(),
        title: r.title.clone(),
        subtitle: r.subtitle.clone(),
        image: r.image.clone(),
        pinned: false,
    }
}

pub fn install(app: &App) -> Vec<String> {
    let store = app.global::<Store>();
    let raw: Vec<Recent> = std::fs::read_to_string(recent_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    SEARCH.with(|s| {
        store.set_search_tracks(ModelRc::from(s.tracks.clone()));
        store.set_search_grid(ModelRc::from(s.grid.clone()));
        store.set_search_sections(ModelRc::from(s.sections.clone()));
        store.set_browse_tiles(ModelRc::from(s.tiles.clone()));
        store.set_recent_searches(ModelRc::from(s.recent.clone()));
        store.set_category_tracks(ModelRc::from(s.category_tracks.clone()));
        store.set_category_sections(ModelRc::from(s.category_sections.clone()));
        *s.recent_raw.borrow_mut() = raw;
    });
    rebuild_recent()
}

fn rebuild_recent() -> Vec<String> {
    SEARCH.with(|s| {
        let mut keys = Vec::new();
        let cards: Vec<CardData> = s.recent_raw.borrow().iter().map(|r| card_data(&to_card(r), &mut keys)).collect();
        s.recent.set_vec(cards);
        keys
    })
}

fn save_recent() {
    SEARCH.with(|s| {
        if let Ok(json) = serde_json::to_string(&*s.recent_raw.borrow()) {
            let _ = std::fs::create_dir_all(Config::data_dir());
            let _ = std::fs::write(recent_path(), json);
        }
    });
}

pub fn remember(card: &CardData) -> Vec<String> {
    if card.uri.is_empty() {
        return Vec::new();
    }
    let image = card.image_url.split('#').next().filter(|u| !u.is_empty()).map(str::to_owned);
    let entry = Recent {
        id: card.id.to_string(),
        uri: card.uri.to_string(),
        kind: card.kind.to_string(),
        title: card.title.to_string(),
        subtitle: card.subtitle.to_string(),
        image,
    };
    SEARCH.with(|s| {
        let mut raw = s.recent_raw.borrow_mut();
        raw.retain(|r| r.uri != entry.uri);
        raw.insert(0, entry);
        raw.truncate(RECENT_LIMIT);
    });
    save_recent();
    rebuild_recent()
}

pub fn remember_track(track: &TrackData) -> Vec<String> {
    remember(&CardData {
        id: track.id.clone(),
        uri: track.uri.clone(),
        kind: "track".into(),
        title: track.title.clone(),
        subtitle: format!("Song • {}", track.artists).into(),
        image_url: track.image_url.clone(),
        ..Default::default()
    })
}

pub fn forget(uri: &str) {
    SEARCH.with(|s| s.recent_raw.borrow_mut().retain(|r| r.uri != uri));
    save_recent();
    rebuild_recent();
}

pub fn clear_recent() {
    SEARCH.with(|s| s.recent_raw.borrow_mut().clear());
    save_recent();
    rebuild_recent();
}

fn sections_model(sections: &[Section], keys: &mut Vec<String>) -> (Vec<SectionData>, Vec<Rc<VecModel<CardData>>>) {
    let mut rows = Vec::new();
    let data = sections
        .iter()
        .map(|s| {
            let model = Rc::new(VecModel::from(s.items.iter().map(|c| card_data(c, keys)).collect::<Vec<_>>()));
            rows.push(model.clone());
            SectionData {
                eyebrow: s.eyebrow.clone().unwrap_or_default().into(),
                title: s.title.as_str().into(),
                items: ModelRc::from(model),
            }
        })
        .collect();
    (data, rows)
}

pub fn clear_results(app: &App) {
    let store = app.global::<Store>();
    store.set_has_top(false);
    store.set_search_empty(false);
    store.set_search_loading(false);
    SEARCH.with(|s| {
        s.tracks.set_vec(Vec::new());
        s.grid.set_vec(Vec::new());
        s.sections.set_vec(Vec::new());
        s.rows.borrow_mut().clear();
        s.track_uris.borrow_mut().clear();
    });
}

pub fn set_results(app: &App, results: &Results) -> Vec<String> {
    let store = app.global::<Store>();
    if store.get_search_query().trim() != results.query || store.get_search_filter() != results.filter.as_str() {
        return Vec::new();
    }
    let mut keys = Vec::new();
    match &results.top {
        Some(top) => {
            let mut card = card_data(top, &mut keys);
            for prefix in ["Podcast • ", "Playlist • ", "Album • ", "Single • ", "Episode • "] {
                if let Some(rest) = card.subtitle.strip_prefix(prefix) {
                    card.subtitle = rest.into();
                }
            }
            store.set_search_top(card);
            store.set_has_top(true);
        }
        None => store.set_has_top(false),
    }
    let tracks: Vec<TrackData> = results.tracks.iter().enumerate().map(|(i, t)| track_data(i, t, true, false)).collect();
    keys.extend(tracks.iter().filter(|t| !t.image_url.is_empty()).map(|t| t.image_url.to_string()));
    let grid: Vec<CardData> = results.grid.iter().map(|c| card_data(c, &mut keys)).collect();
    let (sections, rows) = sections_model(&results.sections, &mut keys);
    let empty = results.top.is_none() && tracks.is_empty() && grid.is_empty() && sections.is_empty();

    SEARCH.with(|s| {
        *s.track_uris.borrow_mut() = results.tracks.iter().map(|t| t.uri.clone()).collect();
        s.tracks.set_vec(tracks);
        s.grid.set_vec(grid);
        s.sections.set_vec(sections);
        *s.rows.borrow_mut() = rows;
    });
    store.set_search_empty(empty);
    store.set_search_loading(false);
    keys
}

pub fn has_tiles() -> bool {
    SEARCH.with(|s| s.tiles.row_count() > 0)
}

fn parse_color(hex: &str) -> Color {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x2a2a2a);
    Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub fn set_tiles(tiles: &[Tile]) -> Vec<String> {
    let mut keys = Vec::new();
    let data: Vec<TileData> = tiles
        .iter()
        .map(|t| {
            let key = t.image.as_deref().map(|u| images::key(u, 240)).unwrap_or_default();
            if !key.is_empty() {
                keys.push(key.clone());
            }
            TileData {
                key: t.key.as_str().into(),
                title: t.title.as_str().into(),
                color: parse_color(&t.color),
                art: state::cached(&key),
                image_url: key.into(),
            }
        })
        .collect();
    SEARCH.with(|s| s.tiles.set_vec(data));
    keys
}

pub fn begin_category(app: &App, key: &str) {
    let store = app.global::<Store>();
    let tile = SEARCH.with(|s| s.tiles.iter().find(|t| t.key == key)).unwrap_or_default();
    store.set_category(tile);
    store.set_category_loading(true);
    SEARCH.with(|s| {
        s.category_tracks.set_vec(Vec::new());
        s.category_sections.set_vec(Vec::new());
        s.category_rows.borrow_mut().clear();
        s.category_uris.borrow_mut().clear();
    });
}

pub fn set_category(app: &App, key: &str, category: &Category) -> Vec<String> {
    let store = app.global::<Store>();
    let mut current = store.get_category();
    if !current.key.is_empty() && current.key != key {
        return Vec::new();
    }
    current.key = key.into();
    current.title = category.title.as_str().into();
    current.color = parse_color(&category.color);
    store.set_category(current);

    let mut keys = Vec::new();
    let tracks: Vec<TrackData> = category.tracks.iter().enumerate().map(|(i, t)| track_data(i, t, true, false)).collect();
    keys.extend(tracks.iter().filter(|t| !t.image_url.is_empty()).map(|t| t.image_url.to_string()));
    let (sections, rows) = sections_model(&category.sections, &mut keys);
    SEARCH.with(|s| {
        *s.category_uris.borrow_mut() = category.tracks.iter().map(|t| t.uri.clone()).collect();
        s.category_tracks.set_vec(tracks);
        s.category_sections.set_vec(sections);
        *s.category_rows.borrow_mut() = rows;
    });
    store.set_category_loading(false);
    keys
}

pub fn search_uris() -> Vec<String> {
    SEARCH.with(|s| s.track_uris.borrow().clone())
}

pub fn category_uris() -> Vec<String> {
    SEARCH.with(|s| s.category_uris.borrow().clone())
}

fn patch_cards(model: &VecModel<CardData>, key: &str, image: &Image) {
    for i in 0..model.row_count() {
        if let Some(mut row) = model.row_data(i)
            && row.image_url == key {
                row.art = image.clone();
                model.set_row_data(i, row);
            }
    }
}

fn patch_tracks(model: &VecModel<TrackData>, key: &str, image: &Image) {
    for i in 0..model.row_count() {
        if let Some(mut row) = model.row_data(i)
            && row.image_url == key {
                row.art = image.clone();
                model.set_row_data(i, row);
            }
    }
}

pub fn apply_image(app: &App, key: &str, image: &Image) {
    let store = app.global::<Store>();
    let mut top = store.get_search_top();
    if top.image_url == key {
        top.art = image.clone();
        store.set_search_top(top);
    }
    let mut category = store.get_category();
    if category.image_url == key {
        category.art = image.clone();
        store.set_category(category);
    }
    SEARCH.with(|s| {
        patch_tracks(&s.tracks, key, image);
        patch_tracks(&s.category_tracks, key, image);
        patch_cards(&s.grid, key, image);
        patch_cards(&s.recent, key, image);
        for row in s.rows.borrow().iter().chain(s.category_rows.borrow().iter()) {
            patch_cards(row, key, image);
        }
        for i in 0..s.tiles.row_count() {
            if let Some(mut tile) = s.tiles.row_data(i)
                && tile.image_url == key {
                    tile.art = image.clone();
                    s.tiles.set_row_data(i, tile);
                }
        }
    });
}
