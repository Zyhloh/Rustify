use crate::models::{SavedState, TrackInfo};
use crate::pages::{self, Route};
use crate::{App, Credits, Menu, NamedItem, PickerItem, Player, Toast, TrackData, ctx::Ctx, images, models, state};
use serde_json::json;
use slint::{ComponentHandle, Image, Model, ModelRc, Timer, TimerMode, VecModel};
use std::{cell::RefCell, rc::Rc, time::Duration};

struct MenuState {
    track: RefCell<TrackData>,
    info: RefCell<Option<TrackInfo>>,
    query: RefCell<String>,
    playlists: Rc<VecModel<PickerItem>>,
    toast: Timer,
    sleep: Timer,
    sleep_at_end: RefCell<Option<String>>,
}

thread_local! {
    static MENU: MenuState = MenuState {
        track: RefCell::new(TrackData::default()),
        info: RefCell::new(None),
        query: RefCell::new(String::new()),
        playlists: Rc::new(VecModel::default()),
        toast: Timer::default(),
        sleep: Timer::default(),
        sleep_at_end: RefCell::new(None),
    };
}

pub fn install(app: &App) {
    MENU.with(|m| app.global::<Menu>().set_playlists(ModelRc::from(m.playlists.clone())));
}

pub fn toast(app: &App, text: &str) {
    let toast = app.global::<Toast>();
    toast.set_text(text.into());
    toast.set_shown(true);
    let weak = app.as_weak();
    MENU.with(|m| {
        m.toast.start(TimerMode::SingleShot, Duration::from_millis(2800), move || {
            if let Some(app) = weak.upgrade() {
                app.global::<Toast>().set_shown(false);
            }
        })
    });
}

pub fn open(app: &App, ctx: &Ctx, track: TrackData, x: f32, y: f32) {
    let menu = app.global::<Menu>();
    let id = track.id.to_string();
    let liked = pages::saved_for(&id).map(|s| s.liked).unwrap_or(false);

    menu.set_track_id(track.id.clone());
    menu.set_liked(liked);
    menu.set_artists(ModelRc::from(Rc::new(VecModel::from(
        track
            .artists
            .split(", ")
            .map(|name| NamedItem { id: "".into(), name: name.into() })
            .collect::<Vec<_>>(),
    ))));
    let header = app.global::<crate::Store>().get_collection();
    let removable = matches!(pages::current(), Route::Collection { ref kind, ref id } if kind == "playlist" && *id == header.id.as_str())
        && header.owner_is_me
        && pages::track_in_view(&track.uri);
    menu.set_removable(removable);
    menu.set_sub("".into());
    menu.set_x(x);
    menu.set_y(y);
    menu.set_open(true);

    MENU.with(|m| {
        *m.track.borrow_mut() = track;
        *m.info.borrow_mut() = None;
        m.query.borrow_mut().clear();
    });
    rebuild_playlists(app);

    let c = ctx.clone();
    let track_id = id.clone();
    ctx.spawn(async move {
        let info_path = format!("/track/{track_id}");
        let saved_path = format!("/saved/{track_id}");
        let (info, saved, options) = tokio::join!(
            c.api.get::<TrackInfo>(&info_path),
            c.api.get::<SavedState>(&saved_path),
            c.api.get::<Vec<models::PlaylistOption>>("/playlists/editable"),
        );
        let loader = c.clone();
        c.ui(move |app| {
            let menu = app.global::<Menu>();
            if menu.get_track_id() != track_id.as_str() {
                return;
            }
            if let Ok(saved) = saved {
                menu.set_liked(saved.liked);
            }
            if let Ok(options) = options {
                pages::set_options(options);
                loader.load_images(rebuild_playlists(app));
            }
            if let Ok(info) = info {
                menu.set_artists(ModelRc::from(Rc::new(VecModel::from(
                    info.artists
                        .iter()
                        .map(|a| NamedItem { id: a.id.as_str().into(), name: a.name.as_str().into() })
                        .collect::<Vec<_>>(),
                ))));
                MENU.with(|m| *m.info.borrow_mut() = Some(info));
            }
        });
    });
}

fn rebuild_playlists(_app: &App) -> Vec<String> {
    MENU.with(|m| {
        let query = m.query.borrow().to_lowercase();
        let mut keys = Vec::new();
        let rows: Vec<PickerItem> = pages::options()
            .iter()
            .filter(|o| query.is_empty() || o.name.to_lowercase().contains(&query))
            .map(|o| {
                let key = o.image.as_deref().map(|u| images::key(u, 64)).unwrap_or_default();
                if !key.is_empty() {
                    keys.push(key.clone());
                }
                PickerItem {
                    id: o.id.as_str().into(),
                    kind: "playlist".into(),
                    title: o.name.as_str().into(),
                    art: state::cached(&key),
                    image_url: key.into(),
                    ..Default::default()
                }
            })
            .collect();
        m.playlists.set_vec(rows);
        keys
    })
}

pub fn apply_image(key: &str, image: &Image) {
    MENU.with(|m| {
        for i in 0..m.playlists.row_count() {
            if let Some(mut row) = m.playlists.row_data(i)
                && row.image_url == key {
                    row.art = image.clone();
                    m.playlists.set_row_data(i, row);
                }
        }
    });
}

pub fn search(app: &App, query: &str) {
    MENU.with(|m| *m.query.borrow_mut() = query.to_owned());
    rebuild_playlists(app);
}

pub fn close(app: &App) {
    let menu = app.global::<Menu>();
    menu.set_open(false);
    menu.set_sub("".into());
}

fn current() -> (TrackData, Option<TrackInfo>) {
    MENU.with(|m| (m.track.borrow().clone(), m.info.borrow().clone()))
}

pub fn add_to(app: &App, ctx: &Ctx, playlist: &str) {
    let (track, _) = current();
    close(app);
    let name = pages::options().into_iter().find(|o| o.id == playlist).map(|o| o.name).unwrap_or_default();
    ctx.update_saved(track.id.to_string(), json!({ "add": [playlist] }));
    toast(app, &format!("Added to {name}"));
}

pub fn create(app: &App, ctx: &Ctx) {
    let (track, _) = current();
    close(app);
    let name = pages::next_playlist_name();
    ctx.create_playlist(track.id.to_string(), name.clone());
    toast(app, &format!("Added to {name}"));
}

pub fn action(app: &App, ctx: &Ctx, action: &str) {
    let (track, info) = current();
    let id = track.id.to_string();
    let uri = track.uri.to_string();
    close(app);

    match action {
        "like" => {
            let liked = !app.global::<Menu>().get_liked();
            ctx.update_saved(id, json!({ "liked": liked }));
            toast(app, if liked { "Added to Liked Songs" } else { "Removed from Liked Songs" });
        }
        "queue" => {
            ctx.command("/player/queue", json!({ "uri": uri }));
            toast(app, "Added to queue");
        }
        "remove" => {
            let header = app.global::<crate::Store>().get_collection();
            pages::remove_track(&uri);
            ctx.update_saved(id, json!({ "remove": [header.id.to_string()] }));
            toast(app, &format!("Removed from {}", header.title));
        }
        "radio" => {
            let player = app.global::<Player>();
            player.set_context_uri("".into());
            player.set_context_title(format!("{} Radio", track.title).into());
            ctx.command("/player/play", json!({ "context_uri": uri, "radio": true }));
        }
        "album" => {
            let album_id = info.as_ref().map(|i| i.album_id.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| track.album_id.to_string());
            if !album_id.is_empty() {
                ctx.navigate(app, Route::Collection { kind: "album".into(), id: album_id });
            }
        }
        "credits" => match info {
            Some(info) => show_credits(app, &info),
            None => toast(app, "Credits are still loading"),
        },
        "copy-link" => copy(app, &format!("https://open.spotify.com/track/{id}"), "Link copied to clipboard"),
        "copy-embed" => copy(
            app,
            &format!(
                "<iframe style=\"border-radius:12px\" src=\"https://open.spotify.com/embed/track/{id}\" width=\"100%\" height=\"152\" frameBorder=\"0\" allow=\"autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture\" loading=\"lazy\"></iframe>"
            ),
            "Embed code copied to clipboard",
        ),
        other => {
            if let Some(artist) = other.strip_prefix("artist:") {
                if !artist.is_empty() {
                    ctx.navigate(app, Route::Artist { id: artist.to_owned() });
                }
            } else if let Some(sleep) = other.strip_prefix("sleep:") {
                set_sleep(app, ctx, sleep);
            }
        }
    }
}

pub fn show_credits(app: &App, info: &TrackInfo) {
    let credits = app.global::<Credits>();
    credits.set_title(info.name.as_str().into());
    credits.set_album(info.album.as_str().into());
    credits.set_release(info.release.as_str().into());
    credits.set_label(info.label.as_str().into());
    credits.set_artists(ModelRc::from(Rc::new(VecModel::from(
        info.artists
            .iter()
            .map(|a| NamedItem { id: a.id.as_str().into(), name: a.name.as_str().into() })
            .collect::<Vec<_>>(),
    ))));
    credits.set_open(true);
}

pub fn copy(app: &App, text: &str, message: &str) {
    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_owned())) {
        Ok(()) => toast(app, message),
        Err(e) => {
            tracing::warn!("clipboard: {e}");
            toast(app, "Couldn't copy to clipboard");
        }
    }
}

fn set_sleep(app: &App, ctx: &Ctx, key: &str) {
    let menu = app.global::<Menu>();
    MENU.with(|m| {
        m.sleep.stop();
        *m.sleep_at_end.borrow_mut() = None;
    });

    match key {
        "off" => {
            menu.set_sleep("".into());
            toast(app, "Sleep timer turned off");
        }
        "end" => {
            let track = app.global::<Player>().get_track_id().to_string();
            MENU.with(|m| *m.sleep_at_end.borrow_mut() = Some(track));
            menu.set_sleep("End of track".into());
            toast(app, "Your sleep timer is set for the end of this track");
        }
        minutes => {
            let Ok(minutes) = minutes.parse::<u64>() else { return };
            let label = if minutes == 60 { "1 hour".to_string() } else { format!("{minutes} min") };
            menu.set_sleep(label.as_str().into());
            toast(app, &format!("Your sleep timer is set for {label}"));
            let (weak, c) = (app.as_weak(), ctx.clone());
            MENU.with(|m| {
                m.sleep.start(TimerMode::SingleShot, Duration::from_secs(minutes * 60), move || {
                    if let Some(app) = weak.upgrade() {
                        sleep_now(&app, &c);
                    }
                })
            });
        }
    }
}

fn sleep_now(app: &App, ctx: &Ctx) {
    app.global::<Menu>().set_sleep("".into());
    MENU.with(|m| *m.sleep_at_end.borrow_mut() = None);
    let player = app.global::<Player>();
    if player.get_playing() {
        player.set_playing(false);
        ctx.command("/player/pause", json!({}));
    }
    toast(app, "Sleep timer ended");
}

pub fn track_changed(app: &App, ctx: &Ctx, new_track: &str) {
    let due = MENU.with(|m| m.sleep_at_end.borrow().as_deref().is_some_and(|t| t != new_track));
    if due {
        sleep_now(app, ctx);
    }
}
