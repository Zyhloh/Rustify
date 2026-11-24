use crate::models::{Profile, TrackRow};
use crate::pages::{card_data, track_data};
use crate::{App, CardData, ProfileData, Store, TrackData, images, state};
use slint::{ComponentHandle, Image, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

struct AccountState {
    playlists: Rc<VecModel<CardData>>,
    artists: Rc<VecModel<CardData>>,
    tracks: Rc<VecModel<TrackData>>,
    history: Rc<VecModel<TrackData>>,
    track_uris: RefCell<Vec<String>>,
    history_uris: RefCell<Vec<String>>,
}

thread_local! {
    static ACCOUNT: AccountState = AccountState {
        playlists: Rc::new(VecModel::default()),
        artists: Rc::new(VecModel::default()),
        tracks: Rc::new(VecModel::default()),
        history: Rc::new(VecModel::default()),
        track_uris: RefCell::new(Vec::new()),
        history_uris: RefCell::new(Vec::new()),
    };
}

pub fn install(app: &App) {
    let store = app.global::<Store>();
    ACCOUNT.with(|a| {
        store.set_profile_playlists(ModelRc::from(a.playlists.clone()));
        store.set_profile_artists(ModelRc::from(a.artists.clone()));
        store.set_profile_tracks(ModelRc::from(a.tracks.clone()));
        store.set_history(ModelRc::from(a.history.clone()));
    });
    store.set_app_version(env!("CARGO_PKG_VERSION").into());
}

pub fn begin_profile(app: &App) {
    let store = app.global::<Store>();
    store.set_profile(ProfileData::default());
    store.set_profile_loading(true);
    ACCOUNT.with(|a| {
        a.playlists.set_vec(Vec::new());
        a.artists.set_vec(Vec::new());
        a.tracks.set_vec(Vec::new());
        a.track_uris.borrow_mut().clear();
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

pub fn set_profile(app: &App, profile: &Profile) -> Vec<String> {
    let store = app.global::<Store>();
    let mut keys = Vec::new();
    let key = profile.image.as_deref().map(|u| images::key(u, 480)).unwrap_or_default();
    if !key.is_empty() {
        keys.push(key.clone());
    }
    store.set_profile(ProfileData {
        id: profile.id.as_str().into(),
        name: profile.name.as_str().into(),
        art: state::cached(&key),
        image_url: key.into(),
        followers: profile.followers as i32,
        following: profile.following as i32,
        playlists: profile.playlists.len() as i32,
        is_me: profile.is_me,
    });
    let playlists: Vec<CardData> = profile.playlists.iter().map(|c| card_data(c, &mut keys)).collect();
    let artists: Vec<CardData> = profile.top_artists.iter().map(|c| card_data(c, &mut keys)).collect();
    let tracks = rows(&profile.top_tracks, &mut keys);
    ACCOUNT.with(|a| {
        *a.track_uris.borrow_mut() = profile.top_tracks.iter().map(|t| t.uri.clone()).collect();
        a.playlists.set_vec(playlists);
        a.artists.set_vec(artists);
        a.tracks.set_vec(tracks);
    });
    store.set_profile_loading(false);
    keys
}

pub fn set_history(app: &App, list: &[TrackRow]) -> Vec<String> {
    let mut keys = Vec::new();
    let data = rows(list, &mut keys);
    ACCOUNT.with(|a| {
        *a.history_uris.borrow_mut() = list.iter().map(|t| t.uri.clone()).collect();
        a.history.set_vec(data);
    });
    app.global::<Store>().set_history_loading(false);
    keys
}

pub fn profile_uris() -> Vec<String> {
    ACCOUNT.with(|a| a.track_uris.borrow().clone())
}

pub fn history_uris() -> Vec<String> {
    ACCOUNT.with(|a| a.history_uris.borrow().clone())
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
    let mut profile = store.get_profile();
    if profile.image_url == key {
        profile.art = image.clone();
        store.set_profile(profile);
    }
    ACCOUNT.with(|a| {
        patch_cards(&a.playlists, key, image);
        patch_cards(&a.artists, key, image);
        patch_tracks(&a.tracks, key, image);
        patch_tracks(&a.history, key, image);
    });
}
