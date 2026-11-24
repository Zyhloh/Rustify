use crate::ctx::{Ctx, url_encode};
use crate::pages::{self, Route, track_data};
use crate::{App, Store, TrackData, menu::toast};
use serde_json::json;
use slint::{ComponentHandle, Image, Model, ModelRc, VecModel};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

struct FindState {
    results: Rc<VecModel<TrackData>>,
    generation: Cell<u64>,
    playlist: RefCell<String>,
}

thread_local! {
    static FIND: FindState = FindState {
        results: Rc::new(VecModel::default()),
        generation: Cell::new(0),
        playlist: RefCell::new(String::new()),
    };
}

pub fn install(app: &App) {
    FIND.with(|f| app.global::<Store>().set_find_results(ModelRc::from(f.results.clone())));
}

pub fn reset(app: &App) {
    FIND.with(|f| {
        f.generation.set(f.generation.get() + 1);
        f.results.set_vec(Vec::new());
        f.playlist.borrow_mut().clear();
    });
    let store = app.global::<Store>();
    store.set_find_query("".into());
    store.set_find_loading(false);
}

pub fn search(app: &App, ctx: &Ctx, query: String) {
    let generation = FIND.with(|f| {
        f.generation.set(f.generation.get() + 1);
        *f.playlist.borrow_mut() = app.global::<Store>().get_collection().id.to_string();
        f.generation.get()
    });
    let query = query.trim().to_owned();
    let store = app.global::<Store>();
    if query.is_empty() {
        FIND.with(|f| f.results.set_vec(Vec::new()));
        store.set_find_loading(false);
        return;
    }
    store.set_find_loading(true);
    let c = ctx.clone();
    ctx.spawn(async move {
        tokio::time::sleep(Duration::from_millis(280)).await;
        if c.ui_value(|_| FIND.with(|f| f.generation.get())).await.ok() != Some(generation) {
            return;
        }
        let path = format!("/search?q={}&filter=songs", url_encode(&query));
        let result = c.api.get::<crate::search::Results>(&path).await;
        let loader = c.clone();
        c.ui(move |app| {
            if FIND.with(|f| f.generation.get()) != generation {
                return;
            }
            let store = app.global::<Store>();
            store.set_find_loading(false);
            let Ok(results) = result else {
                FIND.with(|f| f.results.set_vec(Vec::new()));
                return;
            };
            let rows: Vec<TrackData> = results.tracks.iter().take(10).enumerate().map(|(i, t)| track_data(i, t, true, false)).collect();
            let keys: Vec<String> = rows.iter().filter(|r| !r.image_url.is_empty()).map(|r| r.image_url.to_string()).collect();
            FIND.with(|f| f.results.set_vec(rows));
            loader.load_images(keys);
        });
    });
}

pub fn add(app: &App, ctx: &Ctx, track: TrackData) {
    let playlist = app.global::<Store>().get_collection().id.to_string();
    if playlist.is_empty() || track.id.is_empty() {
        return;
    }
    FIND.with(|f| {
        if let Some(i) = (0..f.results.row_count()).find(|&i| f.results.row_data(i).is_some_and(|r| r.id == track.id)) {
            f.results.remove(i);
        }
    });
    let title = app.global::<Store>().get_collection().title.to_string();
    toast(app, &format!("Added to {title}"));
    let c = ctx.clone();
    let id = track.id.to_string();
    ctx.spawn(async move {
        if let Err(e) = c.api.post_json::<crate::models::SavedState>(&format!("/saved/{id}"), json!({ "add": [playlist] })).await {
            tracing::warn!("find add: {e}");
            c.ui(|app| toast(app, "Couldn't add that song. Try again in a moment"));
        }
        let route = Route::Collection { kind: "playlist".into(), id: playlist };
        if c.ui_value(|_| pages::current()).await.ok().as_ref() == Some(&route) {
            c.load_route(route);
        }
        c.reload_library().await;
    });
}

pub fn apply_image(key: &str, image: &Image) {
    FIND.with(|f| {
        for i in 0..f.results.row_count() {
            if let Some(mut row) = f.results.row_data(i)
                && row.image_url == key {
                    row.art = image.clone();
                    f.results.set_row_data(i, row);
                }
        }
    });
}
