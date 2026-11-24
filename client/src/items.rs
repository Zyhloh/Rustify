use crate::menu::{copy, toast};
use crate::pages::{self, Route};
use crate::{App, CardData, Confirm, ItemMenu, MenuEntry, PlaylistEdit, Store, ctx::Ctx, state};
use serde_json::json;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

thread_local! {
    static TARGET: RefCell<CardData> = RefCell::new(CardData::default());
}

struct Icons;

impl Icons {
    const QUEUE: &str = "M4 6.5 H20 M4 11.5 H12.5 M4 16.5 H12.5 M15.5 11 V19.5 L20.5 15.25 Z";
    const ADD: &str = "M21 12 A9 9 0 1 1 3 12 A9 9 0 1 1 21 12 Z M12 8 V16 M8 12 H16";
    const SAVED: &str = "M21 12 A9 9 0 1 1 3 12 A9 9 0 1 1 21 12 Z M8 12.25 L10.75 15 L16 9.5";
    const EDIT: &str = "M4.5 19.5 H8.25 L19 8.75 L15.25 5 L4.5 15.75 Z M13.25 7 L17 10.75";
    const DELETE: &str = "M4.5 7 H19.5 M9.5 7 V4.5 H14.5 V7 M6.5 7 L7.5 19.5 H16.5 L17.5 7 M10.25 10.5 V16 M13.75 10.5 V16";
    const PERSON: &str = "M16 8 A4 4 0 1 1 8 8 A4 4 0 1 1 16 8 Z M4.5 20 C5.5 16.5 8.5 14.5 12 14.5 C15.5 14.5 18.5 16.5 19.5 20";
    const LINK: &str = "M10.5 13.5 A3.75 3.75 0 0 0 15.8 13.5 L18.6 10.7 A3.75 3.75 0 0 0 13.3 5.4 L12 6.7 M13.5 10.5 A3.75 3.75 0 0 0 8.2 10.5 L5.4 13.3 A3.75 3.75 0 0 0 10.7 18.6 L12 17.3";
}

fn entry(id: &str, label: &str, icon: &str, separator: bool) -> MenuEntry {
    MenuEntry { id: id.into(), label: label.into(), icon: icon.into(), separator, checked: false }
}

fn owned(app: &App, card: &CardData) -> bool {
    if card.kind != "playlist" {
        return false;
    }
    let collection = app.global::<Store>().get_collection();
    if collection.uri == card.uri && collection.owner_is_me {
        return true;
    }
    let me = app.global::<Store>().get_user_name();
    pages::options().iter().any(|o| o.id == card.id.as_str()) || (!me.is_empty() && card.subtitle == format!("Playlist • {me}").as_str())
}

fn entries(app: &App, card: &CardData) -> Vec<MenuEntry> {
    let saved = state::in_library(&card.uri);
    let mut out = Vec::new();
    match card.kind.as_str() {
        "playlist" | "album" => {
            out.push(entry("queue", "Add to queue", Icons::QUEUE, false));
            if owned(app, card) {
                out.push(entry("edit", "Edit details", Icons::EDIT, true));
                out.push(entry("delete", "Delete", Icons::DELETE, false));
            } else if saved {
                out.push(entry("unsave", "Remove from Your Library", Icons::SAVED, true));
            } else {
                out.push(entry("save", "Add to Your Library", Icons::ADD, true));
            }
            let what = if card.kind == "album" { "album" } else { "playlist" };
            out.push(entry("copy-link", &format!("Copy link to {what}"), Icons::LINK, true));
        }
        "collection" => out.push(entry("queue", "Add to queue", Icons::QUEUE, false)),
        "artist" => {
            out.push(if saved || pages::is_following(app, &card.id) {
                entry("unsave", "Unfollow", Icons::PERSON, false)
            } else {
                entry("save", "Follow", Icons::PERSON, false)
            });
            out.push(entry("copy-link", "Copy link to artist", Icons::LINK, true));
        }
        "show" => {
            out.push(if saved {
                entry("unsave", "Unfollow", Icons::SAVED, false)
            } else {
                entry("save", "Follow", Icons::ADD, false)
            });
            out.push(entry("copy-link", "Copy link to show", Icons::LINK, true));
        }
        "episode" => {
            out.push(entry("queue", "Add to queue", Icons::QUEUE, false));
            let store = app.global::<Store>();
            out.push(if store.get_page() == "show" && store.get_show().kind == "episodes" {
                entry("unsave", "Remove from Your Episodes", Icons::SAVED, false)
            } else {
                entry("save", "Add to Your Episodes", Icons::ADD, false)
            });
            out.push(entry("copy-link", "Copy link to episode", Icons::LINK, true));
        }
        "track" => {
            out.push(entry("queue", "Add to queue", Icons::QUEUE, false));
            out.push(entry("copy-link", "Copy link to song", Icons::LINK, true));
        }
        _ => {}
    }
    out
}

fn show(app: &App, list: Vec<MenuEntry>, x: f32, y: f32) {
    let menu = app.global::<ItemMenu>();
    menu.set_entries(ModelRc::from(Rc::new(VecModel::from(list))));
    menu.set_uri("".into());
    menu.set_x(x);
    menu.set_y(y);
    menu.set_open(true);
}

fn sort_entry(id: String, label: &str, checked: bool, separator: bool) -> MenuEntry {
    MenuEntry { id: id.into(), label: label.into(), icon: "".into(), separator, checked }
}

pub fn open_track_sort(app: &App, x: f32, y: f32) {
    let (current, _) = pages::sort_state();
    let liked = app.global::<Store>().get_collection().kind == "collection";
    let list = pages::SORTS
        .iter()
        .map(|(key, label)| {
            let label = if *key == "custom" && liked { "Recently added" } else { label };
            sort_entry(format!("track-sort:{key}"), label, current == *key, false)
        })
        .collect();
    show(app, list, x, y);
}

pub fn open_library_sort(app: &App, x: f32, y: f32) {
    let current = state::library_sort();
    let list = state::LIBRARY_SORTS
        .iter()
        .map(|(key, label)| sort_entry(format!("library-sort:{key}"), label, current == *key, false))
        .collect();
    show(app, list, x, y);
}

pub fn edit_open_collection(app: &App, ctx: &Ctx) {
    let header = app.global::<Store>().get_collection();
    let card = CardData {
        id: header.id.clone(),
        uri: header.uri.clone(),
        kind: "playlist".into(),
        title: header.title.clone(),
        image_url: header.image_url.clone(),
        art: header.art.clone(),
        ..Default::default()
    };
    TARGET.with(|t| *t.borrow_mut() = card);
    pick(app, ctx, "edit");
}

pub fn open_preset_menu(app: &App, x: f32, y: f32) {
    let current = app.global::<crate::Prefs>().get_eq_preset();
    let list = crate::prefs::PRESETS
        .iter()
        .map(|(name, _)| sort_entry(format!("eq-preset:{name}"), name, current == *name, false))
        .collect();
    show(app, list, x, y);
}

pub fn open_app_menu(app: &App, x: f32, y: f32) {
    let collapsed = app.global::<crate::Shell>().get_library_collapsed();
    let plain = |id: &str, label: &str, separator: bool| sort_entry(format!("app:{id}"), label, false, separator);
    let list = vec![
        plain("new-playlist", "Create playlist", false),
        plain("news", "What's New", false),
        plain("history", "Recently played", false),
        plain("library", if collapsed { "Expand Your Library" } else { "Collapse Your Library" }, true),
        plain("mini", "Open Miniplayer", false),
        plain("fullscreen", "Full screen", false),
        plain("settings", "Settings", true),
        plain("support", "Support", false),
        plain("logout", "Log out", true),
        plain("exit", "Exit", false),
    ];
    show(app, list, x, y);
}

fn app_action(app: &App, key: &str) {
    let store = app.global::<Store>();
    let shell = app.global::<crate::Shell>();
    match key {
        "new-playlist" => store.invoke_new_playlist(),
        "news" => store.invoke_open_news(),
        "history" => store.invoke_open_history(),
        "library" => shell.set_library_collapsed(!shell.get_library_collapsed()),
        "mini" => shell.invoke_enter_mini(),
        "fullscreen" => shell.invoke_toggle_fullscreen(),
        "settings" => store.invoke_open_settings(),
        "support" => store.invoke_open_external("https://support.spotify.com".into()),
        "logout" => store.invoke_logout(),
        "exit" => shell.invoke_close(),
        _ => {}
    }
}

pub fn open(app: &App, card: CardData, x: f32, y: f32) {
    let list = entries(app, &card);
    if list.is_empty() {
        return;
    }
    let menu = app.global::<ItemMenu>();
    menu.set_entries(ModelRc::from(Rc::new(VecModel::from(list))));
    menu.set_uri(card.uri.clone());
    menu.set_x(x);
    menu.set_y(y);
    menu.set_open(true);
    TARGET.with(|t| *t.borrow_mut() = card);
}

pub fn pick(app: &App, ctx: &Ctx, action: &str) {
    app.global::<ItemMenu>().set_open(false);
    if let Some(key) = action.strip_prefix("track-sort:") {
        ctx.load_images(pages::sort_tracks(app, key));
        return;
    }
    if let Some(key) = action.strip_prefix("library-sort:") {
        state::sort_library(app, key);
        return;
    }
    if let Some(name) = action.strip_prefix("eq-preset:") {
        crate::prefs::apply_preset(app, name);
        app.global::<crate::Prefs>().invoke_audio_changed();
        return;
    }
    if let Some(key) = action.strip_prefix("app:") {
        app_action(app, key);
        return;
    }
    let card = TARGET.with(|t| t.borrow().clone());
    let uri = card.uri.to_string();
    match action {
        "queue" => {
            let target = if card.kind == "collection" { "collection".to_owned() } else { uri };
            let c = ctx.clone();
            let weak = app.as_weak();
            ctx.spawn(async move {
                let done = c.api.post_json::<serde_json::Value>("/player/queue", json!({ "uri": target })).await;
                let message = match done {
                    Ok(_) => "Added to queue",
                    Err(e) => {
                        tracing::warn!("queue: {e}");
                        "Couldn't add to queue"
                    }
                };
                let _ = weak.upgrade_in_event_loop(move |app| toast(&app, message));
            });
        }
        "save" | "unsave" => {
            let saved = action == "save";
            if card.kind == "artist" {
                if pages::is_following(app, &card.id) != saved {
                    ctx.toggle_follow(app, card.id.to_string());
                } else {
                    ctx.save_item(app, uri, saved);
                }
            } else {
                if card.kind == "show" && app.global::<Store>().get_show().uri == card.uri {
                    crate::podcast::set_saved(app, saved);
                }
                if card.kind == "episode" {
                    ctx.save_episode(app, uri, saved);
                } else {
                    ctx.save_item(app, uri, saved);
                }
            }
        }
        "copy-link" => {
            let mut parts = uri.splitn(3, ':').skip(1);
            match (parts.next(), parts.next()) {
                (Some(kind), Some(id)) => copy(app, &format!("https://open.spotify.com/{kind}/{id}"), "Link copied to clipboard"),
                _ => toast(app, "No link for this item"),
            }
        }
        "edit" => {
            let edit = app.global::<PlaylistEdit>();
            let header = app.global::<Store>().get_collection();
            let description = if header.id == card.id { header.description.clone() } else { "".into() };
            edit.set_id(card.id.clone());
            edit.set_name(card.title.clone());
            edit.set_description(description);
            edit.set_art(if card.art.size().width > 0 { card.art.clone() } else { state::cached(&card.image_url) });
            edit.set_error("".into());
            edit.set_busy(false);
            edit.set_open(true);
        }
        "delete" => {
            let confirm = app.global::<Confirm>();
            confirm.set_title("Delete from Your Library?".into());
            confirm.set_body(format!("This will delete {} from Your Library.", card.title).into());
            confirm.set_action_label("Delete".into());
            confirm.set_open(true);
        }
        _ => {}
    }
}

pub fn save_edit(app: &App, ctx: &Ctx, name: String, description: String) {
    let edit = app.global::<PlaylistEdit>();
    let name = name.trim().to_owned();
    if name.is_empty() {
        edit.set_error("Playlist name is required.".into());
        return;
    }
    if name.chars().count() > 100 {
        edit.set_error("Name must be 100 characters or fewer.".into());
        return;
    }
    if description.chars().count() > 300 {
        edit.set_error("Description must be 300 characters or fewer.".into());
        return;
    }
    let id = edit.get_id().to_string();
    edit.set_busy(true);
    edit.set_error("".into());
    let c = ctx.clone();
    let weak = app.as_weak();
    ctx.spawn(async move {
        let result = c
            .api
            .post(&format!("/playlist/{id}/edit"), json!({ "name": name, "description": description.trim() }))
            .await;
        let ok = result.is_ok();
        if let Err(e) = &result {
            tracing::warn!("edit playlist: {e}");
        }
        let _ = weak.upgrade_in_event_loop(move |app| {
            let edit = app.global::<PlaylistEdit>();
            edit.set_busy(false);
            if !ok {
                edit.set_error("Couldn't save changes. Try again.".into());
                return;
            }
            edit.set_open(false);
            let store = app.global::<Store>();
            let mut header = store.get_collection();
            if header.id == id.as_str() {
                header.title = name.as_str().into();
                header.description = description.trim().into();
                store.set_collection(header);
            }
            toast(&app, "Details saved");
        });
        c.reload_library().await;
    });
}

pub fn confirm_delete(app: &App, ctx: &Ctx) {
    app.global::<Confirm>().set_open(false);
    let card = TARGET.with(|t| t.borrow().clone());
    let id = card.id.to_string();
    if id.is_empty() {
        return;
    }
    if matches!(pages::current(), Route::Collection { id: ref open, .. } if *open == id) {
        pages::push(app, Route::Home);
    }
    let c = ctx.clone();
    ctx.spawn(async move {
        let message = match c.api.post(&format!("/playlist/{id}/delete"), json!({})).await {
            Ok(_) => "Removed from Your Library",
            Err(e) => {
                tracing::warn!("delete playlist: {e}");
                "Couldn't delete that playlist. Try again in a moment"
            }
        };
        c.ui(move |app| toast(app, message));
        c.reload_library().await;
    });
}
