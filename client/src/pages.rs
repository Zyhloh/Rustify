use crate::models::{ArtistPage, Collection, PlaylistOption, SavedState, TrackRow};
use crate::{App, ArtistPageData, CardData, CollectionData, Picker, PickerItem, Player, SectionData, Store, TrackData, images, state};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    rc::Rc,
};

const BATCH: usize = 80;
const TRACK_PX: u32 = 80;
const HEADER_PX: u32 = 480;
const LIKED: &str = "liked";

#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    Home,
    Collection { kind: String, id: String },
    Artist { id: String },
    Search,
    Category { key: String },
    Lyrics,
    Profile { user: String },
    Settings,
    History,
    ShowAll { title: String },
    Show { id: String },
    News,
    Plugin { plugin: String, id: String },
    Extensions,
    Extension { id: String },
}

struct Pages {
    history: RefCell<Vec<Route>>,
    pos: Cell<usize>,
    tracks_all: RefCell<Vec<TrackData>>,
    base: RefCell<Vec<TrackData>>,
    view: RefCell<(String, String, bool)>,
    tracks: Rc<VecModel<TrackData>>,
    saved: RefCell<(String, SavedState)>,
    options: RefCell<Vec<PlaylistOption>>,
    original: RefCell<HashSet<String>>,
    checked: RefCell<HashSet<String>>,
    query: RefCell<String>,
    picker_saved: Rc<VecModel<PickerItem>>,
    picker_others: Rc<VecModel<PickerItem>>,
    artist_top: Rc<VecModel<TrackData>>,
    artist_rows: RefCell<Vec<Rc<VecModel<CardData>>>>,
    shelves: RefCell<Vec<(SectionData, bool)>>,
}

thread_local! {
    static PAGES: Pages = Pages {
        history: RefCell::new(vec![Route::Home]),
        pos: Cell::new(0),
        tracks_all: RefCell::new(Vec::new()),
        base: RefCell::new(Vec::new()),
        view: RefCell::new((String::new(), "custom".into(), false)),
        tracks: Rc::new(VecModel::default()),
        saved: RefCell::new((String::new(), SavedState::default())),
        options: RefCell::new(Vec::new()),
        original: RefCell::new(HashSet::new()),
        checked: RefCell::new(HashSet::new()),
        query: RefCell::new(String::new()),
        picker_saved: Rc::new(VecModel::default()),
        picker_others: Rc::new(VecModel::default()),
        artist_top: Rc::new(VecModel::default()),
        artist_rows: RefCell::new(Vec::new()),
        shelves: RefCell::new(Vec::new()),
    };
}

pub fn install(app: &App) {
    PAGES.with(|p| {
        app.global::<Store>().set_tracks(ModelRc::from(p.tracks.clone()));
        app.global::<Store>().set_artist_top(ModelRc::from(p.artist_top.clone()));
        let picker = app.global::<Picker>();
        picker.set_saved(ModelRc::from(p.picker_saved.clone()));
        picker.set_others(ModelRc::from(p.picker_others.clone()));
    });
}

pub fn track_in_view(uri: &str) -> bool {
    PAGES.with(|p| p.tracks_all.borrow().iter().any(|t| t.uri == uri))
}

pub fn remove_track(uri: &str) {
    PAGES.with(|p| {
        p.tracks_all.borrow_mut().retain(|t| t.uri != uri);
        p.base.borrow_mut().retain(|t| t.uri != uri);
        if let Some(i) = (0..p.tracks.row_count()).find(|&i| p.tracks.row_data(i).is_some_and(|t| t.uri == uri)) {
            p.tracks.remove(i);
        }
    });
}

pub fn current() -> Route {
    current_route()
}

fn current_route() -> Route {
    PAGES.with(|p| p.history.borrow()[p.pos.get()].clone())
}

fn show(app: &App) -> Route {
    let route = current_route();
    let store = app.global::<Store>();
    PAGES.with(|p| {
        store.set_can_back(p.pos.get() > 0);
        store.set_can_forward(p.pos.get() + 1 < p.history.borrow().len());
    });
    match &route {
        Route::Home => store.set_page("home".into()),
        Route::Collection { kind, id } => {
            crate::find::reset(app);
            PAGES.with(|p| {
                p.tracks_all.borrow_mut().clear();
                p.base.borrow_mut().clear();
                *p.view.borrow_mut() = (String::new(), "custom".into(), false);
                p.tracks.set_vec(Vec::new());
            });
            store.set_collection_query("".into());
            store.set_collection_sort(if kind == "collection" { "Recently added".into() } else { "Custom order".into() });
            let known = if kind == "collection" { crate::state::library_card("liked") } else { crate::state::library_card(id) };
            let mut header = CollectionData { kind: kind.as_str().into(), ..Default::default() };
            if let Some(card) = known {
                let (label, rest) = card.subtitle.split_once(" • ").unwrap_or((card.subtitle.as_str(), ""));
                header.title = card.title.clone();
                header.label = label.into();
                header.art = card.art.clone();
                header.image_url = card.image_url.clone();
                if kind == "collection" {
                    header.owner = store.get_user_name();
                    header.owner_art = store.get_user_art();
                    header.owner_is_me = true;
                    header.meta = rest.into();
                } else {
                    header.owner = rest.into();
                }
            }
            store.set_collection(header);
            store.set_collection_color(if kind == "collection" { Color::from_rgb_u8(0x50, 0x38, 0xa0) } else { Color::from_rgb_u8(0x3a, 0x3a, 0x3a) });
            store.set_collection_loading(true);
            store.set_page("collection".into());
        }
        Route::Artist { .. } => {
            PAGES.with(|p| {
                p.artist_top.set_vec(Vec::new());
                p.artist_rows.borrow_mut().clear();
            });
            store.set_artist_page(ArtistPageData::default());
            store.set_artist_shelves(ModelRc::default());
            store.set_artist_color(Color::from_rgb_u8(0x3a, 0x3a, 0x3a));
            store.set_artist_loading(true);
            store.set_page("artist".into());
        }
        Route::Search => store.set_page("search".into()),
        Route::Lyrics => store.set_page("lyrics".into()),
        Route::Profile { .. } => {
            crate::account::begin_profile(app);
            store.set_page("profile".into());
        }
        Route::Settings => store.set_page("settings".into()),
        Route::Extensions => store.set_page("extensions".into()),
        Route::Extension { id } => {
            crate::plugins::select(app, id);
            store.set_page("extension".into());
        }
        Route::Plugin { plugin, id } => {
            crate::plugins::show_page(app, plugin, id);
            store.set_page("plugin".into());
        }
        Route::History => {
            store.set_history_loading(true);
            store.set_page("history".into());
        }
        Route::News => {
            crate::news::begin(app);
            store.set_page("news".into());
        }
        Route::Show { id } => {
            crate::podcast::begin(app, id);
            store.set_page("show".into());
        }
        Route::ShowAll { title } => {
            let found = PAGES.with(|p| p.shelves.borrow().iter().rev().find(|(s, _)| s.title.as_str() == title.as_str()).cloned());
            if let Some((section, search)) = found {
                store.set_show_all(section);
                store.set_show_all_search(search);
            }
            store.set_page("show-all".into());
        }
        Route::Category { key } => {
            crate::search::begin_category(app, key);
            store.set_page("category".into());
        }
    }
    if !matches!(route, Route::Plugin { .. }) {
        crate::plugins::left_page();
    }
    crate::plugins::page_changed(&store.get_page());
    route
}

pub fn show_all(app: &App, section: SectionData, search: bool) -> Route {
    let title = section.title.to_string();
    PAGES.with(|p| {
        let mut shelves = p.shelves.borrow_mut();
        shelves.retain(|(s, _)| s.title.as_str() != title);
        shelves.push((section, search));
        if shelves.len() > 16 {
            shelves.remove(0);
        }
    });
    push(app, Route::ShowAll { title })
}

pub fn push(app: &App, route: Route) -> Route {
    PAGES.with(|p| {
        let mut history = p.history.borrow_mut();
        if history[p.pos.get()] != route {
            history.truncate(p.pos.get() + 1);
            history.push(route);
            p.pos.set(history.len() - 1);
        }
    });
    show(app)
}

pub fn back(app: &App) -> Option<Route> {
    let moved = PAGES.with(|p| {
        let can = p.pos.get() > 0;
        if can {
            p.pos.set(p.pos.get() - 1);
        }
        can
    });
    moved.then(|| show(app))
}

pub fn forward(app: &App) -> Option<Route> {
    let moved = PAGES.with(|p| {
        let can = p.pos.get() + 1 < p.history.borrow().len();
        if can {
            p.pos.set(p.pos.get() + 1);
        }
        can
    });
    moved.then(|| show(app))
}

pub fn set_collection(app: &App, route: &Route, c: &Collection) -> Vec<String> {
    if &current_route() != route {
        return Vec::new();
    }
    let store = app.global::<Store>();
    let header_key = c.image.as_deref().map(|u| images::key(u, HEADER_PX)).unwrap_or_default();
    let owner_key = c.owner_image.as_deref().map(|u| images::key(u, 64)).unwrap_or_default();
    let mut extra = Vec::new();
    if !owner_key.is_empty() {
        extra.push(owner_key.clone());
    }
    let more: Vec<CardData> = c.more.iter().map(|card| card_data(card, &mut extra)).collect();
    let more_title = if c.more.is_empty() { String::new() } else { format!("More by {}", c.owner.split(", ").next().unwrap_or_default()) };
    store.set_collection_more(SectionData {
        eyebrow: "".into(),
        title: more_title.into(),
        items: ModelRc::from(Rc::new(VecModel::from(more))),
    });
    store.set_collection_footer(ModelRc::from(Rc::new(VecModel::from(
        c.footer.iter().map(|f| slint::SharedString::from(f.as_str())).collect::<Vec<_>>(),
    ))));

    store.set_collection(CollectionData {
        id: c.id.as_str().into(),
        uri: c.uri.as_str().into(),
        context: c.context.as_str().into(),
        kind: c.kind.as_str().into(),
        label: c.label.as_str().into(),
        title: c.title.as_str().into(),
        description: c.description.as_str().into(),
        owner: c.owner.as_str().into(),
        owner_is_me: c.owner_is_me,
        owner_id: c.owner_id.as_str().into(),
        owner_art: state::cached(&owner_key),
        owner_image_url: owner_key.as_str().into(),
        meta: c.meta.as_str().into(),
        art: state::cached(&header_key),
        image_url: header_key.as_str().into(),
        show_album: c.show_album,
        show_added: c.show_added,
        show_added_by: c.show_added_by,
        show_art: c.show_art,
    });
    store.set_collection_loading(false);

    let rows: Vec<TrackData> = c
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| track_data(i, t, c.show_art, c.show_added_by))
        .collect();

    PAGES.with(|p| *p.base.borrow_mut() = rows);
    let mut keys = apply_view(app);
    if !header_key.is_empty() {
        keys.insert(0, header_key);
    }
    keys.extend(extra);
    keys
}

fn seconds(duration: &str) -> u64 {
    duration.split(':').filter_map(|p| p.parse::<u64>().ok()).fold(0, |acc, p| acc * 60 + p)
}

fn apply_view(app: &App) -> Vec<String> {
    PAGES.with(|p| {
        let (query, sort, desc) = p.view.borrow().clone();
        let needle = query.trim().to_lowercase();
        let mut rows: Vec<TrackData> = p
            .base
            .borrow()
            .iter()
            .filter(|t| {
                needle.is_empty()
                    || t.title.to_lowercase().contains(&needle)
                    || t.artists.to_lowercase().contains(&needle)
                    || t.album.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        match sort.as_str() {
            "title" => rows.sort_by_key(|t| t.title.to_lowercase()),
            "artist" => rows.sort_by_key(|t| t.artists.to_lowercase()),
            "album" => rows.sort_by_key(|t| t.album.to_lowercase()),
            "duration" => rows.sort_by_key(|t| seconds(&t.duration)),
            _ => {}
        }
        if desc {
            rows.reverse();
        }
        for (i, row) in rows.iter_mut().enumerate() {
            row.index = i as i32 + 1;
        }
        *p.tracks_all.borrow_mut() = rows;
        p.tracks.set_vec(Vec::new());
    });
    load_more(app)
}

pub fn filter_tracks(app: &App, query: &str) -> Vec<String> {
    PAGES.with(|p| p.view.borrow_mut().0 = query.to_owned());
    apply_view(app)
}

pub const SORTS: [(&str, &str); 5] = [("custom", "Custom order"), ("title", "Title"), ("artist", "Artist"), ("album", "Album"), ("duration", "Duration")];

pub fn sort_state() -> (String, bool) {
    PAGES.with(|p| {
        let view = p.view.borrow();
        (view.1.clone(), view.2)
    })
}

pub fn sort_tracks(app: &App, key: &str) -> Vec<String> {
    let label = SORTS.iter().find(|(k, _)| *k == key).map(|(_, l)| *l).unwrap_or("Custom order");
    PAGES.with(|p| {
        let mut view = p.view.borrow_mut();
        view.2 = view.1 == key && key != "custom" && !view.2;
        view.1 = key.to_owned();
    });
    let store = app.global::<Store>();
    let custom = if store.get_collection().kind == "collection" { "Recently added" } else { "Custom order" };
    store.set_collection_sort(if key == "custom" { custom.into() } else { label.into() });
    apply_view(app)
}

pub fn track_data(i: usize, t: &TrackRow, art: bool, added_by: bool) -> TrackData {
    let key = if art {
        t.image.as_deref().map(|u| images::key(u, TRACK_PX)).unwrap_or_default()
    } else {
        String::new()
    };
    TrackData {
        index: i as i32 + 1,
        id: t.id.as_str().into(),
        uri: t.uri.as_str().into(),
        album_id: t.album_id.as_str().into(),
        title: t.name.as_str().into(),
        artists: t.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ").into(),
        album: t.album.as_str().into(),
        added: if added_by { t.added_by.as_str() } else { t.added.as_str() }.into(),
        duration: t.duration.as_str().into(),
        explicit: t.explicit,
        art: state::cached(&key),
        image_url: key.into(),
    }
}

pub fn card_data(c: &crate::models::Card, keys: &mut Vec<String>) -> CardData {
    let key = c.image.as_deref().map(|u| images::key(u, state::CARD_PX)).unwrap_or_default();
    if !key.is_empty() {
        keys.push(key.clone());
    }
    CardData {
        id: c.id.as_str().into(),
        uri: c.uri.as_str().into(),
        kind: c.kind.as_str().into(),
        title: c.title.as_str().into(),
        subtitle: c.subtitle.as_str().into(),
        pinned: c.pinned,
        art: state::cached(&key),
        image_url: key.into(),
    }
}

pub fn set_artist(app: &App, route: &Route, a: &ArtistPage) -> Vec<String> {
    if &current_route() != route {
        return Vec::new();
    }
    let store = app.global::<Store>();
    let key = a.image.as_deref().map(|u| images::key(u, 960)).unwrap_or_default();
    let mut keys: Vec<String> = Vec::new();
    if !key.is_empty() {
        keys.push(key.clone());
    }

    store.set_artist_page(ArtistPageData {
        id: a.id.as_str().into(),
        uri: a.uri.as_str().into(),
        name: a.name.as_str().into(),
        art: state::cached(&key),
        image_url: key.into(),
        followers: a.followers.map(|f| f.to_string()).unwrap_or_default().into(),
        following: a.following,
        listeners: a.listeners.as_str().into(),
        verified: a.verified,
    });

    let top: Vec<TrackData> = a.top.iter().enumerate().map(|(i, t)| track_data(i, t, true, false)).collect();
    keys.extend(top.iter().filter(|t| !t.image_url.is_empty()).map(|t| t.image_url.to_string()));

    let mut rows = Vec::new();
    let mut shelves = Vec::new();
    for (title, items) in [("Albums", &a.albums), ("Singles and EPs", &a.singles)] {
        if items.is_empty() {
            continue;
        }
        let cards: Vec<CardData> = items.iter().map(|c| card_data(c, &mut keys)).collect();
        let model = Rc::new(VecModel::from(cards));
        rows.push(model.clone());
        shelves.push(SectionData { eyebrow: "".into(), title: title.into(), items: ModelRc::from(model) });
    }

    PAGES.with(|p| {
        p.artist_top.set_vec(top);
        *p.artist_rows.borrow_mut() = rows;
    });
    store.set_artist_shelves(ModelRc::from(Rc::new(VecModel::from(shelves))));
    store.set_artist_loading(false);
    keys
}

pub fn load_more(_app: &App) -> Vec<String> {
    PAGES.with(|p| {
        let all = p.tracks_all.borrow();
        let start = p.tracks.row_count();
        let end = (start + BATCH).min(all.len());
        let mut keys = Vec::new();
        for row in &all[start..end] {
            if !row.image_url.is_empty() {
                keys.push(row.image_url.to_string());
            }
            p.tracks.push(row.clone());
        }
        keys
    })
}

pub fn apply_image(app: &App, key: &str, image: &Image, accent: [f32; 3]) {
    let store = app.global::<Store>();
    let mut header = store.get_collection();
    if header.image_url == key {
        header.art = image.clone();
        store.set_collection(header);
        store.set_collection_color(state::shade(accent, 0.46));
    }
    let mut header = store.get_collection();
    if header.owner_image_url == key {
        header.owner_art = image.clone();
        store.set_collection(header);
    }
    let more = store.get_collection_more().items;
    for i in 0..more.row_count() {
        if let Some(mut card) = more.row_data(i)
            && card.image_url == key {
                card.art = image.clone();
                more.set_row_data(i, card);
            }
    }

    let mut artist = store.get_artist_page();
    if artist.image_url == key {
        artist.art = image.clone();
        store.set_artist_page(artist);
        store.set_artist_color(state::shade(accent, 0.4));
    }

    PAGES.with(|p| {
        for row in p.tracks_all.borrow_mut().iter_mut().filter(|r| r.image_url == key) {
            row.art = image.clone();
        }
        for row in p.base.borrow_mut().iter_mut().filter(|r| r.image_url == key) {
            row.art = image.clone();
        }
        for row in p.artist_rows.borrow().iter() {
            for i in 0..row.row_count() {
                if let Some(mut card) = row.row_data(i)
                    && card.image_url == key {
                        card.art = image.clone();
                        row.set_row_data(i, card);
                    }
            }
        }
        for model in [&p.tracks, &p.artist_top] {
            for i in 0..model.row_count() {
                if let Some(mut row) = model.row_data(i)
                    && row.image_url == key {
                        row.art = image.clone();
                        model.set_row_data(i, row);
                    }
            }
        }
        for model in [&p.picker_saved, &p.picker_others] {
            for i in 0..model.row_count() {
                if let Some(mut row) = model.row_data(i)
                    && row.image_url == key {
                        row.art = image.clone();
                        model.set_row_data(i, row);
                    }
            }
        }
    });
}

pub fn set_saved(app: &App, track_id: &str, saved: SavedState) {
    let player = app.global::<Player>();
    if player.get_track_id() != track_id {
        return;
    }
    player.set_saved(saved.liked || !saved.playlists.is_empty());
    PAGES.with(|p| *p.saved.borrow_mut() = (track_id.to_owned(), saved));
}

pub fn saved_for(track_id: &str) -> Option<SavedState> {
    PAGES.with(|p| {
        let saved = p.saved.borrow();
        (saved.0 == track_id).then(|| saved.1.clone())
    })
}

fn liked_subtitle(app: &App) -> String {
    let library = app.global::<Store>().get_library();
    library
        .row_data(0)
        .filter(|c| c.kind == "collection")
        .and_then(|c| c.subtitle.split(" • ").nth(1).map(str::to_owned))
        .unwrap_or_default()
}

pub fn open_picker(app: &App, options: Vec<PlaylistOption>) -> Vec<String> {
    let track = app.global::<Player>().get_track_id().to_string();
    let saved = saved_for(&track).unwrap_or_default();
    let mut original: HashSet<String> = saved.playlists.into_iter().collect();
    if saved.liked {
        original.insert(LIKED.into());
    }
    PAGES.with(|p| {
        *p.options.borrow_mut() = options;
        *p.checked.borrow_mut() = original.clone();
        *p.original.borrow_mut() = original;
        p.query.borrow_mut().clear();
    });
    let keys = rebuild(app);
    let picker = app.global::<Picker>();
    picker.set_dirty(false);
    picker.set_open(true);
    keys
}

fn rebuild(app: &App) -> Vec<String> {
    let liked_sub = liked_subtitle(app);
    PAGES.with(|p| {
        let query = p.query.borrow().to_lowercase();
        let original = p.original.borrow();
        let checked = p.checked.borrow();
        let mut keys = Vec::new();

        let mut rows = vec![PickerItem {
            id: LIKED.into(),
            kind: "liked".into(),
            title: "Liked Songs".into(),
            subtitle: liked_sub.as_str().into(),
            checked: checked.contains(LIKED),
            ..Default::default()
        }];
        for o in p.options.borrow().iter() {
            let key = o.image.as_deref().map(|u| images::key(u, TRACK_PX)).unwrap_or_default();
            if !key.is_empty() {
                keys.push(key.clone());
            }
            rows.push(PickerItem {
                id: o.id.as_str().into(),
                kind: "playlist".into(),
                title: o.name.as_str().into(),
                subtitle: format!("{} song{}", o.total, if o.total == 1 { "" } else { "s" }).into(),
                checked: checked.contains(&o.id),
                art: state::cached(&key),
                image_url: key.into(),
            });
        }

        let (saved, others): (Vec<PickerItem>, Vec<PickerItem>) = rows
            .into_iter()
            .filter(|r| query.is_empty() || r.title.to_lowercase().contains(&query))
            .partition(|r| original.contains(r.id.as_str()));
        p.picker_saved.set_vec(saved);
        p.picker_others.set_vec(others);
        keys
    })
}

pub fn toggle(app: &App, id: &str) {
    let dirty = PAGES.with(|p| {
        let mut checked = p.checked.borrow_mut();
        if !checked.remove(id) {
            checked.insert(id.to_owned());
        }
        for model in [&p.picker_saved, &p.picker_others] {
            for i in 0..model.row_count() {
                if let Some(mut row) = model.row_data(i)
                    && row.id == id {
                        row.checked = checked.contains(id);
                        model.set_row_data(i, row);
                    }
            }
        }
        *checked != *p.original.borrow()
    });
    app.global::<Picker>().set_dirty(dirty);
}

pub fn search(app: &App, query: &str) {
    PAGES.with(|p| *p.query.borrow_mut() = query.to_owned());
    rebuild(app);
}

pub struct Diff {
    pub liked: Option<bool>,
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

pub fn picker_diff() -> Diff {
    PAGES.with(|p| {
        let original = p.original.borrow();
        let checked = p.checked.borrow();
        let liked = (original.contains(LIKED) != checked.contains(LIKED)).then(|| checked.contains(LIKED));
        Diff {
            liked,
            add: checked.difference(&original).filter(|id| *id != LIKED).cloned().collect(),
            remove: original.difference(&checked).filter(|id| *id != LIKED).cloned().collect(),
        }
    })
}

pub fn close_picker(app: &App) {
    app.global::<Picker>().set_open(false);
}

pub fn next_playlist_name() -> String {
    let count = PAGES.with(|p| p.options.borrow().len());
    format!("My Playlist #{}", count + 1)
}

pub fn set_following(app: &App, id: &str, following: bool) {
    let store = app.global::<Store>();
    let mut page = store.get_artist_page();
    if page.id == id {
        page.following = following;
        store.set_artist_page(page);
    }
    let mut artist = store.get_artist();
    if artist.id == id {
        artist.following = following;
        store.set_artist(artist);
    }
}

pub fn is_following(app: &App, id: &str) -> bool {
    let store = app.global::<Store>();
    let page = store.get_artist_page();
    if page.id == id {
        return page.following;
    }
    let artist = store.get_artist();
    artist.id == id && artist.following
}

pub fn options() -> Vec<PlaylistOption> {
    PAGES.with(|p| p.options.borrow().clone())
}

pub fn set_options(options: Vec<PlaylistOption>) {
    PAGES.with(|p| *p.options.borrow_mut() = options);
}
