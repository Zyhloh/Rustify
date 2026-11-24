use crate::models::{self, Card, Home};
use crate::{App, ArtistData, CardData, Player, SectionData, Store, images};
use slint::{ComponentHandle, Image, Model, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    time::Instant,
};

pub const CARD_PX: u32 = 360;
pub const ROW_PX: u32 = 112;
pub const HERO_PX: u32 = 660;
pub const COVER_PX: u32 = 720;

#[derive(Default)]
struct Keys {
    user: String,
    art: String,
    art_large: String,
    track: String,
    artist: String,
}

pub struct Models {
    library_all: RefCell<Vec<CardData>>,
    library: Rc<VecModel<CardData>>,
    shortcuts: Rc<VecModel<CardData>>,
    sections: Rc<VecModel<SectionData>>,
    sections_all: RefCell<Vec<(String, SectionData)>>,
    rows: RefCell<Vec<Rc<VecModel<CardData>>>>,
    images: RefCell<HashMap<String, Image>>,
    image_order: RefCell<std::collections::VecDeque<String>>,
    keys: RefCell<Keys>,
    position: Cell<(f32, Option<Instant>)>,
    volume_touched: Cell<Option<Instant>>,
}

thread_local! {
    static MODELS: Models = Models {
        library_all: RefCell::new(Vec::new()),
        library: Rc::new(VecModel::default()),
        shortcuts: Rc::new(VecModel::default()),
        sections: Rc::new(VecModel::default()),
        sections_all: RefCell::new(Vec::new()),
        rows: RefCell::new(Vec::new()),
        images: RefCell::new(HashMap::new()),
        image_order: RefCell::new(std::collections::VecDeque::new()),
        keys: RefCell::new(Keys::default()),
        position: Cell::new((0.0, None)),
        volume_touched: Cell::new(None),
    };
}

pub fn install(app: &App) {
    MODELS.with(|m| {
        let store = app.global::<Store>();
        store.set_library(ModelRc::from(m.library.clone()));
        store.set_shortcuts(ModelRc::from(m.shortcuts.clone()));
        store.set_sections(ModelRc::from(m.sections.clone()));
    });
}

const IMAGE_CAP: usize = 600;

pub fn is_cached(key: &str) -> bool {
    MODELS.with(|m| m.images.borrow().contains_key(key))
}

pub fn cached(key: &str) -> Image {
    MODELS.with(|m| m.images.borrow().get(key).cloned().unwrap_or_default())
}

fn card(c: &Card, px: u32, wanted: &mut Vec<String>) -> CardData {
    let key = c.image.as_deref().map(|u| images::key(u, px)).unwrap_or_default();
    if !key.is_empty() {
        wanted.push(key.clone());
    }
    CardData {
        id: c.id.as_str().into(),
        uri: c.uri.as_str().into(),
        kind: c.kind.as_str().into(),
        title: c.title.as_str().into(),
        subtitle: c.subtitle.as_str().into(),
        pinned: c.pinned,
        art: cached(&key),
        image_url: key.into(),
    }
}

pub fn set_user(app: &App, me: &models::Me) -> Vec<String> {
    let store = app.global::<Store>();
    store.set_user_name(me.display_name.as_str().into());
    store.set_me_email(me.email.as_str().into());
    store.set_me_plan(me.product.as_str().into());
    store.set_me_country(me.country.as_str().into());
    store.set_user_initial(me.display_name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into());
    let key = me.image.as_deref().map(|u| images::key(u, 64)).unwrap_or_default();
    MODELS.with(|m| m.keys.borrow_mut().user = key.clone());
    if key.is_empty() { Vec::new() } else { vec![key] }
}

pub fn set_library(app: &App, items: &[Card]) -> Vec<String> {
    let mut wanted = Vec::new();
    let cards: Vec<CardData> = items.iter().map(|c| card(c, ROW_PX, &mut wanted)).collect();
    MODELS.with(|m| *m.library_all.borrow_mut() = cards);
    filter_library(app, &app.global::<Store>().get_library_filter());
    let player = app.global::<Player>();
    if player.get_context_title().is_empty()
        && let Some(title) = library_title(&player.get_context_uri()) {
            player.set_context_title(title);
        }
    wanted
}

pub const LIBRARY_SORTS: [(&str, &str); 3] = [("recents", "Recents"), ("alpha", "Alphabetical"), ("creator", "Creator")];

thread_local! {
    static LIBRARY_VIEW: RefCell<(String, String)> = RefCell::new((String::new(), "recents".into()));
}

pub fn library_sort() -> String {
    LIBRARY_VIEW.with(|v| v.borrow().1.clone())
}

pub fn sort_library(app: &App, key: &str) {
    LIBRARY_VIEW.with(|v| v.borrow_mut().1 = key.to_owned());
    let label = LIBRARY_SORTS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or("Recents");
    app.global::<Store>().set_library_sort(label.into());
    filter_library(app, &app.global::<Store>().get_library_filter());
}

pub fn search_library(app: &App, query: &str) {
    LIBRARY_VIEW.with(|v| v.borrow_mut().0 = query.trim().to_lowercase());
    filter_library(app, &app.global::<Store>().get_library_filter());
}

fn creator(card: &CardData) -> String {
    card.subtitle.split(" • ").nth(1).unwrap_or_default().to_lowercase()
}

pub fn filter_home(app: &App, filter: &str) {
    let store = app.global::<Store>();
    store.set_home_filter(filter.into());
    MODELS.with(|m| {
        let all = m.sections_all.borrow();
        let wanted = match filter {
            "music" => "music",
            "podcasts" => "podcast",
            "audiobooks" => "audiobook",
            _ => "",
        };
        let shown: Vec<SectionData> = all
            .iter()
            .filter(|(kind, _)| if wanted.is_empty() { kind != "audiobook" } else { kind == wanted })
            .map(|(_, s)| s.clone())
            .collect();
        m.sections.set_vec(shown);
    });
}

pub fn library_card(id: &str) -> Option<CardData> {
    MODELS.with(|m| m.library_all.borrow().iter().find(|c| c.id == id).cloned())
}

pub fn library_title(uri: &str) -> Option<slint::SharedString> {
    if uri.ends_with(":collection") {
        return Some("Liked Songs".into());
    }
    if uri == "spotify:collection:your-episodes" {
        return Some("Your Episodes".into());
    }
    MODELS.with(|m| m.library_all.borrow().iter().find(|c| c.uri == uri).map(|c| c.title.clone()))
}

pub fn in_library(uri: &str) -> bool {
    MODELS.with(|m| m.library_all.borrow().iter().any(|c| c.uri == uri))
}

pub fn filter_library(app: &App, kind: &str) {
    app.global::<Store>().set_library_filter(kind.into());
    let (query, sort) = LIBRARY_VIEW.with(|v| v.borrow().clone());
    MODELS.with(|m| {
        let all = m.library_all.borrow();
        let mut filtered: Vec<CardData> = all
            .iter()
            .filter(|c| {
                kind.is_empty()
                    || c.kind == kind
                    || (kind == "playlist" && (c.kind == "collection" || c.kind == "dj"))
                    || (kind == "podcast" && (c.kind == "episodes" || c.kind == "show"))
            })
            .filter(|c| query.is_empty() || c.title.to_lowercase().contains(&query) || c.subtitle.to_lowercase().contains(&query))
            .cloned()
            .collect();
        match sort.as_str() {
            "alpha" => filtered.sort_by_key(|c| (!c.pinned, c.title.to_lowercase())),
            "creator" => filtered.sort_by_key(|c| (!c.pinned, creator(c), c.title.to_lowercase())),
            _ => {}
        }
        m.library.set_vec(filtered);
    });
}

pub fn set_home(app: &App, home: &Home) -> Vec<String> {
    let mut wanted = Vec::new();
    let store = app.global::<Store>();

    match &home.hero {
        Some(hero) => {
            store.set_hero(card(&hero.card, HERO_PX, &mut wanted));
            store.set_hero_headline(hero.headline.as_str().into());
            store.set_hero_eyebrow(hero.eyebrow.as_str().into());
            store.set_has_hero(true);
        }
        None => store.set_has_hero(false),
    }

    MODELS.with(|m| {
        m.shortcuts.set_vec(
            home.shortcuts.iter().map(|c| card(c, ROW_PX, &mut wanted)).collect::<Vec<_>>(),
        );

        let mut rows = Vec::new();
        let sections: Vec<(String, SectionData)> = home
            .sections
            .iter()
            .map(|s| {
                let row = Rc::new(VecModel::from(
                    s.items.iter().map(|c| card(c, CARD_PX, &mut wanted)).collect::<Vec<_>>(),
                ));
                rows.push(row.clone());
                let data = SectionData {
                    eyebrow: s.eyebrow.clone().unwrap_or_default().into(),
                    title: s.title.as_str().into(),
                    items: ModelRc::from(row),
                };
                (s.kind.clone(), data)
            })
            .collect();
        *m.rows.borrow_mut() = rows;
        *m.sections_all.borrow_mut() = sections;
    });
    filter_home(app, &store.get_home_filter());

    store.set_loading(false);
    wanted
}

fn patch(model: &VecModel<CardData>, key: &str, image: &Image) {
    for i in 0..model.row_count() {
        if let Some(mut row) = model.row_data(i)
            && row.image_url == key {
                row.art = image.clone();
                model.set_row_data(i, row);
            }
    }
}

pub fn apply_image(app: &App, key: &str, image: Image, accent: [f32; 3]) {
    crate::pages::apply_image(app, key, &image, accent);
    crate::menu::apply_image(key, &image);
    crate::search::apply_image(app, key, &image);
    crate::panels::apply_image(app, key, &image);
    crate::account::apply_image(app, key, &image);
    crate::podcast::apply_image(app, key, &image, accent);
    crate::find::apply_image(key, &image);
    crate::news::apply_image(key, &image);
    crate::plugins::apply_image(key, &image);
    let store = app.global::<Store>();
    let player = app.global::<Player>();

    MODELS.with(|m| {
        let mut images = m.images.borrow_mut();
        if images.insert(key.to_owned(), image.clone()).is_none() {
            let mut order = m.image_order.borrow_mut();
            order.push_back(key.to_owned());
            while order.len() > IMAGE_CAP {
                if let Some(old) = order.pop_front() {
                    images.remove(&old);
                }
            }
        }
        drop(images);

        for c in m.library_all.borrow_mut().iter_mut().filter(|c| c.image_url == key) {
            c.art = image.clone();
        }
        patch(&m.library, key, &image);
        patch(&m.shortcuts, key, &image);
        for row in m.rows.borrow().iter() {
            patch(row, key, &image);
        }

        let keys = m.keys.borrow();
        if keys.user == key {
            store.set_user_art(image.clone());
        }
        if keys.art == key {
            player.set_art(image.clone());
        }
        if keys.art_large == key {
            player.set_art_large(image.clone());
            if let Some(color) = art_color(&image) {
                player.set_art_color(color);
            }
        }
        if keys.artist == key {
            let mut artist = store.get_artist();
            artist.art = image.clone();
            store.set_artist(artist);
        }
    });

    let mut hero = store.get_hero();
    if hero.image_url == key {
        hero.art = image;
        store.set_hero(hero);
        store.set_hero_color(shade(accent, 0.34));
        store.set_hero_tint(shade(accent, 0.24));
    }
}

pub fn shade(rgb: [f32; 3], lightness: f32) -> slint::Color {
    let max = rgb[0].max(rgb[1]).max(rgb[2]).max(0.01);
    let mean = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    let [r, g, b] = rgb.map(|c| {
        let vivid = mean + (c - mean) * 1.6;
        ((vivid / max) * lightness).clamp(0.0, 1.0) * 255.0
    });
    slint::Color::from_rgb_u8(r as u8, g as u8, b as u8)
}

pub struct TrackChange {
    pub track_id: String,
    pub artist_id: String,
    pub images: Vec<String>,
}

pub fn apply_player(app: &App, state: &models::PlayerState) -> Option<TrackChange> {
    let player = app.global::<Player>();
    player.set_ready(state.ready);
    player.set_playing(state.is_playing);
    player.set_shuffle(state.shuffle);
    player.set_repeat(state.repeat.as_str().into());
    let touched = MODELS.with(|m| m.volume_touched.get());
    if touched.is_none_or(|t| t.elapsed().as_millis() > 1500) {
        player.set_volume(state.volume as f32 / 100.0);
    }
    player.set_position(state.position_ms as f32);
    MODELS.with(|m| {
        m.position.set((state.position_ms as f32, state.is_playing.then(Instant::now)))
    });

    let Some(track) = &state.track else {
        player.set_has_track(false);
        player.set_duration(0.0);
        player.set_position(0.0);
        MODELS.with(|m| m.keys.borrow_mut().track.clear());
        return None;
    };

    player.set_has_track(true);
    let voice = track.uri.starts_with("spotify:media:");
    player.set_voice(voice);
    player.set_dj(state.context.as_deref() == Some("spotify:playlist:37i9dQZF1EYkqdzj48dyYq") || voice);
    player.set_duration(track.duration_ms as f32);
    if let Some(context) = state.context.as_deref()
        && (player.get_context_uri() != context || player.get_context_title().is_empty()) {
            match library_title(context) {
                Some(title) => {
                    player.set_context_uri(context.into());
                    player.set_context_title(title);
                }
                None if player.get_context_title().is_empty() => player.set_context_uri(context.into()),
                None => {}
            }
        }

    let changed = MODELS.with(|m| m.keys.borrow().track != track.id);
    if !changed {
        return None;
    }

    player.set_title(track.name.as_str().into());
    player.set_track_id(track.id.as_str().into());
    player.set_saved(false);
    player.set_track_uri(track.uri.as_str().into());
    player.set_explicit(track.explicit);
    player.set_artists(
        track.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ").into(),
    );

    let small = track.image_small.as_deref().map(|u| images::key(u, 128)).unwrap_or_default();
    let large = track.image_large.as_deref().map(|u| images::key(u, COVER_PX)).unwrap_or_default();
    player.set_art(cached(&small));
    let large_image = cached(&large);
    if let Some(color) = art_color(&large_image) {
        player.set_art_color(color);
    }
    player.set_art_large(large_image);

    MODELS.with(|m| {
        let mut keys = m.keys.borrow_mut();
        keys.track = track.id.clone();
        keys.art = small.clone();
        keys.art_large = large.clone();
    });

    Some(TrackChange {
        track_id: track.id.clone(),
        artist_id: track.artists.first().map(|a| a.id.clone()).unwrap_or_default(),
        images: [small, large].into_iter().filter(|k| !k.is_empty()).collect(),
    })
}

pub fn set_artist(app: &App, artist: &models::ArtistInfo) -> Vec<String> {
    let key = artist.image.as_deref().map(|u| images::key(u, CARD_PX * 2)).unwrap_or_default();
    MODELS.with(|m| m.keys.borrow_mut().artist = key.clone());
    app.global::<Store>().set_artist(ArtistData {
        id: artist.id.as_str().into(),
        name: artist.name.as_str().into(),
        art: cached(&key),
        image_url: key.clone().into(),
        followers: artist.followers.map(group_digits).unwrap_or_default().into(),
        genres: artist.genres.join(", ").into(),
        following: artist.following,
        loaded: true,
    });
    if key.is_empty() { Vec::new() } else { vec![key] }
}

pub fn tick(app: &App) {
    let (base, since) = MODELS.with(|m| m.position.get());
    if let Some(since) = since {
        let player = app.global::<Player>();
        let pos = (base + since.elapsed().as_millis() as f32).min(player.get_duration());
        player.set_position(pos);
    }
}

pub fn touch_volume() {
    MODELS.with(|m| m.volume_touched.set(Some(Instant::now())));
}

pub fn set_position(ms: f32, playing: bool) {
    MODELS.with(|m| m.position.set((ms, playing.then(Instant::now))));
}

fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn art_color(image: &slint::Image) -> Option<slint::Color> {
    let pixels = image.to_rgba8()?;
    let (w, h) = (pixels.width() as usize, pixels.height() as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let data = pixels.as_slice();
    let step = (w * h / 4096).max(1);
    let (mut r, mut g, mut b, mut weight) = (0f32, 0f32, 0f32, 0f32);
    for p in data.iter().step_by(step) {
        let (pr, pg, pb) = (p.r as f32 / 255.0, p.g as f32 / 255.0, p.b as f32 / 255.0);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let w = 0.15 + (max - min) * 2.0;
        r += pr * w;
        g += pg * w;
        b += pb * w;
        weight += w;
    }
    let (r, g, b) = (r / weight, g / weight, b / weight);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d == 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()) };
    let hue = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let (s, l) = (s.min(0.55), l.clamp(0.2, 0.3));
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((hue / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(slint::Color::from_rgb_u8(to(r1), to(g1), to(b1)))
}
