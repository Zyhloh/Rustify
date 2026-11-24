use crate::{App, CardData, Credits, Menu, Picker, Player, Shell, Store, TrackData, ctx::Ctx, menu, pages, state};
use serde_json::json;
use slint::{
    ComponentHandle,
    winit_030::{WinitWindowAccessor, winit::window::ResizeDirection},
};

pub fn bind(app: &App, ctx: &Ctx) {
    bind_store(app, ctx);
    bind_player(app, ctx);
    bind_picker(app, ctx);
    bind_search(app, ctx);
    bind_panels(app, ctx);
    bind_items(app, ctx);
    bind_account(app, ctx);
    bind_menu(app, ctx);
    bind_shell(app);
}

fn bind_store(app: &App, ctx: &Ctx) {
    let store = app.global::<Store>();

    let c = ctx.clone();
    store.on_login(move || c.command("/auth/login", json!({})));

    let c = ctx.clone();
    store.on_refresh(move || {
        let c2 = c.clone();
        c.spawn(async move { c2.load().await });
    });

    let weak = app.as_weak();
    store.on_filter_library(move |kind| {
        if let Some(app) = weak.upgrade() {
            state::filter_library(&app, &kind);
        }
    });

    let play = {
        let c = ctx.clone();
        let weak = app.as_weak();
        move |card: CardData| {
            if let Some(app) = weak.upgrade() {
                let player = app.global::<Player>();
                player.set_context_uri(card.uri.clone());
                player.set_context_title(card.title.clone());
            }
            c.play(&card.kind, &card.uri);
        }
    };
    store.on_play_card(play);

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_card(move |card| {
        if let Some(app) = weak.upgrade() {
            c.open_card(&app, &card);
        }
    });

    let weak = app.as_weak();
    store.on_go_home(move || {
        if let Some(app) = weak.upgrade() {
            pages::push(&app, pages::Route::Home);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_go_back(move || {
        if let Some(route) = weak.upgrade().and_then(|app| pages::back(&app)) {
            c.load_route(route);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_go_forward(move || {
        if let Some(route) = weak.upgrade().and_then(|app| pages::forward(&app)) {
            c.load_route(route);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_load_more(move || {
        if let Some(app) = weak.upgrade() {
            c.load_images(pages::load_more(&app));
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_play_collection(move || {
        let Some(app) = weak.upgrade() else { return };
        let collection = app.global::<Store>().get_collection();
        let player = app.global::<Player>();
        if player.get_context_uri() == collection.uri && player.get_has_track() {
            player.invoke_toggle();
            return;
        }
        player.set_context_uri(collection.uri.clone());
        player.set_context_title(collection.title.clone());
        c.play(&collection.kind, &collection.context);
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_play_track(move |track: TrackData| {
        let Some(app) = weak.upgrade() else { return };
        let collection = app.global::<Store>().get_collection();
        let player = app.global::<Player>();
        player.set_context_uri(collection.uri.clone());
        player.set_context_title(collection.title.clone());
        c.play_in_context(&collection.context, &track.uri);
    });
}

fn bind_menu(app: &App, ctx: &Ctx) {
    let store = app.global::<Store>();
    let m = app.global::<Menu>();

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_track_menu(move |track, x, y| {
        if let Some(app) = weak.upgrade() {
            menu::open(&app, &c, track, x, y);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    m.on_action(move |action| {
        if let Some(app) = weak.upgrade() {
            menu::action(&app, &c, &action);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    m.on_add_to(move |playlist| {
        if let Some(app) = weak.upgrade() {
            menu::add_to(&app, &c, &playlist);
        }
    });

    let weak = app.as_weak();
    m.on_search(move |query| {
        if let Some(app) = weak.upgrade() {
            menu::search(&app, &query);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    m.on_create(move || {
        if let Some(app) = weak.upgrade() {
            menu::create(&app, &c);
        }
    });

    let weak = app.as_weak();
    m.on_close(move || {
        if let Some(app) = weak.upgrade() {
            menu::close(&app);
        }
    });

    let weak = app.as_weak();
    app.global::<Credits>().on_close(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<Credits>().set_open(false);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_artist(move |id| {
        if let Some(app) = weak.upgrade()
            && !id.is_empty() {
                c.navigate(&app, pages::Route::Artist { id: id.into() });
            }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_toggle_follow(move |id| {
        if let Some(app) = weak.upgrade() {
            c.toggle_follow(&app, id.into());
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_play_artist(move || {
        let Some(app) = weak.upgrade() else { return };
        let artist = app.global::<Store>().get_artist_page();
        let player = app.global::<Player>();
        if player.get_context_uri() == artist.uri && player.get_has_track() {
            player.invoke_toggle();
            return;
        }
        player.set_context_uri(artist.uri.clone());
        player.set_context_title(artist.name.clone());
        c.play("artist", &artist.uri);
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_play_artist_track(move |track: TrackData| {
        let Some(app) = weak.upgrade() else { return };
        let artist = app.global::<Store>().get_artist_page();
        let player = app.global::<Player>();
        player.set_context_uri(artist.uri.clone());
        player.set_context_title(artist.name.clone());
        c.play_in_context(&artist.uri, &track.uri);
    });
}

fn bind_account(app: &App, ctx: &Ctx) {
    let store = app.global::<Store>();

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_profile(move |user| {
        if let Some(app) = weak.upgrade() {
            c.open_profile(&app, user.to_string());
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_settings(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<Store>().set_cache_size(crate::prefs::human_size(crate::prefs::cache_size()).into());
            c.navigate(&app, pages::Route::Settings);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_history(move || {
        if let Some(app) = weak.upgrade() {
            c.navigate(&app, pages::Route::History);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_logout(move || {
        if let Some(app) = weak.upgrade() {
            c.logout(&app);
        }
    });

    store.on_open_external(|url| crate::panels::open_url(&url));

    let weak = app.as_weak();
    store.on_copy_text(move |text| {
        if let Some(app) = weak.upgrade() {
            crate::menu::copy(&app, &text, "Copied to clipboard");
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_save_client_id(move |id| {
        if let Some(app) = weak.upgrade() {
            c.save_client_id(&app, id.to_string());
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_clear_cache(move || {
        if let Some(app) = weak.upgrade() {
            c.clear_cache(&app);
        }
    });

    let c = ctx.clone();
    store.on_play_profile_track(move |track| c.play_uris(crate::account::profile_uris(), &track.uri));

    let c = ctx.clone();
    store.on_play_history_track(move |track| c.play_uris(crate::account::history_uris(), &track.uri));

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_transfer_here(move || {
        if let Some(app) = weak.upgrade() {
            c.transfer_here(&app);
        }
    });

    let weak = app.as_weak();
    app.global::<crate::Prefs>().on_changed(move || {
        if let Some(app) = weak.upgrade() {
            crate::prefs::save(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    app.global::<crate::Prefs>().on_audio_changed(move || {
        if let Some(app) = weak.upgrade() {
            crate::prefs::save(&app);
            c.push_audio(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    app.global::<crate::Prefs>().on_set_band(move |index, value| {
        if let Some(app) = weak.upgrade() {
            crate::prefs::set_band(&app, index.max(0) as usize, value);
            crate::prefs::save(&app);
            c.push_audio(&app);
        }
    });

    let weak = app.as_weak();
    app.global::<crate::Prefs>().on_preset_menu(move |x, y| {
        if let Some(app) = weak.upgrade() {
            crate::items::open_preset_menu(&app, x, y);
        }
    });
}

fn bind_items(app: &App, ctx: &Ctx) {
    let store = app.global::<Store>();

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_new_playlist(move || {
        if let Some(app) = weak.upgrade() {
            c.new_playlist(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_find_search(move |query| {
        if let Some(app) = weak.upgrade() {
            crate::find::search(&app, &c, query.to_string());
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_find_add(move |track| {
        if let Some(app) = weak.upgrade() {
            crate::find::add(&app, &c, track);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_collection_filter(move |query| {
        if let Some(app) = weak.upgrade() {
            c.load_images(pages::filter_tracks(&app, &query));
        }
    });

    let weak = app.as_weak();
    store.on_collection_sort_menu(move |x, y| {
        if let Some(app) = weak.upgrade() {
            crate::items::open_track_sort(&app, x, y);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_show_credits(move || {
        if let Some(app) = weak.upgrade() {
            c.show_credits(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_news(move || {
        if let Some(app) = weak.upgrade() {
            c.navigate(&app, pages::Route::News);
        }
    });

    let weak = app.as_weak();
    store.on_filter_news(move |kind| {
        if let Some(app) = weak.upgrade() {
            crate::news::filter(&app, &kind);
        }
    });

    let weak = app.as_weak();
    store.on_filter_home(move |filter| {
        if let Some(app) = weak.upgrade() {
            state::filter_home(&app, &filter);
        }
    });

    let weak = app.as_weak();
    store.on_library_search(move |query| {
        if let Some(app) = weak.upgrade() {
            state::search_library(&app, &query);
        }
    });

    let weak = app.as_weak();
    store.on_library_sort_menu(move |x, y| {
        if let Some(app) = weak.upgrade() {
            crate::items::open_library_sort(&app, x, y);
        }
    });

    let c = ctx.clone();
    store.on_connect_library(move || c.connect_library());

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_edit_collection(move || {
        if let Some(app) = weak.upgrade() {
            crate::items::edit_open_collection(&app, &c);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_retry_collection(move || {
        if weak.upgrade().is_some() {
            c.load_route(pages::current());
        }
    });

    let weak = app.as_weak();
    store.on_app_menu(move |x, y| {
        if let Some(app) = weak.upgrade() {
            crate::items::open_app_menu(&app, x, y);
        }
    });

    let weak = app.as_weak();
    store.on_card_menu(move |card, x, y| {
        if let Some(app) = weak.upgrade() {
            crate::items::open(&app, card, x, y);
        }
    });

    let menu = app.global::<crate::ItemMenu>();
    let (c, weak) = (ctx.clone(), app.as_weak());
    menu.on_pick(move |action| {
        if let Some(app) = weak.upgrade() {
            crate::items::pick(&app, &c, &action);
        }
    });

    let weak = app.as_weak();
    menu.on_close(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<crate::ItemMenu>().set_open(false);
        }
    });

    let edit = app.global::<crate::PlaylistEdit>();
    let (c, weak) = (ctx.clone(), app.as_weak());
    edit.on_save(move |name, description| {
        if let Some(app) = weak.upgrade() {
            crate::items::save_edit(&app, &c, name.to_string(), description.to_string());
        }
    });

    let weak = app.as_weak();
    edit.on_cancel(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<crate::PlaylistEdit>().set_open(false);
        }
    });

    let confirm = app.global::<crate::Confirm>();
    let (c, weak) = (ctx.clone(), app.as_weak());
    confirm.on_accept(move || {
        if let Some(app) = weak.upgrade() {
            crate::items::confirm_delete(&app, &c);
        }
    });

    let weak = app.as_weak();
    confirm.on_cancel(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<crate::Confirm>().set_open(false);
        }
    });
}

fn bind_panels(app: &App, ctx: &Ctx) {
    let panels = app.global::<crate::Panels>();

    let (c, weak) = (ctx.clone(), app.as_weak());
    panels.on_open_panel(move |name| {
        if let Some(app) = weak.upgrade() {
            c.open_panel(&app, &name);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    panels.on_queue_tab_picked(move |tab| {
        if let Some(app) = weak.upgrade() {
            app.global::<crate::Panels>().set_queue_tab(tab);
            c.refresh_panel(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    panels.on_play_queue_item(move |track, index| {
        c.play_uris(crate::panels::queue_from(index.max(0) as usize), &track.uri);
        if let Some(app) = weak.upgrade() {
            c.refresh_panel(&app);
        }
    });

    let c = ctx.clone();
    panels.on_play_recent(move |track, index| {
        c.play_uris(crate::panels::recent_from(index.max(0) as usize), &track.uri);
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    panels.on_transfer(move |device| {
        if let Some(app) = weak.upgrade() {
            c.transfer(&app, device.to_string());
        }
    });

    panels.on_open_url(|url| crate::panels::open_url(&url));

    let (c, weak) = (ctx.clone(), app.as_weak());
    panels.on_toggle_lyrics(move || {
        if let Some(app) = weak.upgrade() {
            c.toggle_lyrics(&app);
        }
    });
}

fn bind_search(app: &App, ctx: &Ctx) {
    let store = app.global::<Store>();

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_search_focus(move || {
        if let Some(app) = weak.upgrade() {
            c.search_focus(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_search_edited(move |_| {
        if let Some(app) = weak.upgrade() {
            if app.global::<Store>().get_page() != "search" {
                c.search_focus(&app);
            }
            c.run_search(&app, 250);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_show_all({
        let weak = app.as_weak();
        let c = ctx.clone();
        move |section, search| {
            let Some(app) = weak.upgrade() else { return };
            let filter = match section.title.as_str() {
                "Artists" => "artists",
                "Albums" => "albums",
                "Playlists" => "playlists",
                "Podcasts" | "Episodes" => "podcasts",
                _ => "",
            };
            if search && !filter.is_empty() {
                app.global::<Store>().invoke_search_filter_picked(filter.into());
                return;
            }
            let route = pages::show_all(&app, section, search);
            c.load_route(route);
        }
    });

    store.on_open_show({
        let weak = app.as_weak();
        let c = ctx.clone();
        move |id| {
            if let Some(app) = weak.upgrade()
                && !id.is_empty() {
                    c.navigate(&app, pages::Route::Show { id: id.to_string() });
                }
        }
    });

    store.on_play_show({
        let weak = app.as_weak();
        let c = ctx.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            let show = app.global::<Store>().get_show();
            let player = app.global::<Player>();
            let context = if show.kind == "episodes" { "spotify:collection:your-episodes".into() } else { show.uri.clone() };
            if player.get_context_uri() == context && player.get_has_track() {
                player.invoke_toggle();
                return;
            }
            player.set_context_uri(context);
            player.set_context_title(show.title.clone());
            let uris = crate::podcast::uris();
            if show.kind == "episodes" {
                if let Some(first) = uris.first().cloned() {
                    c.play_uris(uris, &first);
                }
            } else {
                c.play("show", &show.uri);
            }
        }
    });

    store.on_play_episode({
        let weak = app.as_weak();
        let c = ctx.clone();
        move |episode| {
            let Some(app) = weak.upgrade() else { return };
            let show = app.global::<Store>().get_show();
            let player = app.global::<Player>();
            if player.get_track_uri() == episode.uri && player.get_has_track() {
                player.invoke_toggle();
                return;
            }
            if show.kind == "episodes" {
                player.set_context_uri("spotify:collection:your-episodes".into());
                player.set_context_title(show.title.clone());
                c.play_uris(crate::podcast::uris(), &episode.uri);
            } else {
                player.set_context_uri(show.uri.clone());
                player.set_context_title(show.title.clone());
                c.play_context(&show.uri, &episode.uri);
            }
        }
    });

    store.on_toggle_show_saved({
        let weak = app.as_weak();
        let c = ctx.clone();
        move || {
            let Some(app) = weak.upgrade() else { return };
            let show = app.global::<Store>().get_show();
            if show.uri.is_empty() || show.kind != "show" {
                return;
            }
            crate::podcast::set_saved(&app, !show.saved);
            c.save_item(&app, show.uri.to_string(), !show.saved);
        }
    });

    store.on_search_filter_picked(move |filter| {
        if let Some(app) = weak.upgrade() {
            app.global::<Store>().set_search_filter(filter);
            crate::search::clear_results(&app);
            c.run_search(&app, 0);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_search_result(move |card| {
        if let Some(app) = weak.upgrade() {
            let store = app.global::<Store>();
            store.set_blur_search_request(store.get_blur_search_request() + 1);
            c.remember(&card);
            c.open_card(&app, &card);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_play_search_result(move |card| {
        if let Some(app) = weak.upgrade() {
            c.remember(&card);
            let player = app.global::<Player>();
            player.set_context_uri(card.uri.clone());
            player.set_context_title(card.title.clone());
            c.play(&card.kind, &card.uri);
        }
    });

    let c = ctx.clone();
    store.on_play_search_track(move |track| {
        c.load_images(crate::search::remember_track(&track));
        c.play_uris(crate::search::search_uris(), &track.uri);
    });

    let c = ctx.clone();
    store.on_play_category_track(move |track| {
        c.play_uris(crate::search::category_uris(), &track.uri);
    });

    store.on_remove_recent(|card| crate::search::forget(&card.uri));
    store.on_clear_recent(crate::search::clear_recent);

    let (c, weak) = (ctx.clone(), app.as_weak());
    store.on_open_tile(move |tile| {
        if let Some(app) = weak.upgrade() {
            c.navigate(&app, pages::Route::Category { key: tile.key.to_string() });
        }
    });
}

fn bind_picker(app: &App, ctx: &Ctx) {
    let picker = app.global::<Picker>();

    let weak = app.as_weak();
    picker.on_toggle(move |id| {
        if let Some(app) = weak.upgrade() {
            pages::toggle(&app, &id);
        }
    });

    let weak = app.as_weak();
    picker.on_search(move |query| {
        if let Some(app) = weak.upgrade() {
            pages::search(&app, &query);
        }
    });

    let weak = app.as_weak();
    picker.on_cancel(move || {
        if let Some(app) = weak.upgrade() {
            pages::close_picker(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    picker.on_done(move || {
        if let Some(app) = weak.upgrade() {
            c.picker_done(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    picker.on_create(move || {
        if let Some(app) = weak.upgrade() {
            c.picker_create(&app);
        }
    });
}

fn bind_player(app: &App, ctx: &Ctx) {
    let player = app.global::<Player>();

    let (c, weak) = (ctx.clone(), app.as_weak());
    player.on_toggle(move || {
        if let Some(app) = weak.upgrade() {
            let p = app.global::<Player>();
            let playing = !p.get_playing();
            p.set_playing(playing);
            state::set_position(p.get_position(), playing);
        }
        c.command("/player/toggle", json!({}));
    });

    let c = ctx.clone();
    player.on_next(move || c.command("/player/next", json!({})));

    let c = ctx.clone();
    player.on_prev(move || c.command("/player/prev", json!({})));

    let (c, weak) = (ctx.clone(), app.as_weak());
    player.on_seek(move |ms| {
        if let Some(app) = weak.upgrade() {
            let p = app.global::<Player>();
            p.set_position(ms);
            state::set_position(ms, p.get_playing());
        }
        c.command("/player/seek", json!({ "position_ms": ms.max(0.0) as u32 }));
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    player.on_set_volume(move |v| {
        if let Some(app) = weak.upgrade() {
            app.global::<Player>().set_volume(v);
        }
        c.set_volume(v, false);
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    player.on_commit_volume(move |v| {
        if let Some(app) = weak.upgrade() {
            app.global::<Player>().set_volume(v);
        }
        c.set_volume(v, true);
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    player.on_toggle_shuffle(move || {
        if let Some(app) = weak.upgrade() {
            let p = app.global::<Player>();
            let on = !p.get_shuffle();
            p.set_shuffle(on);
            c.command("/player/shuffle", json!({ "state": on }));
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    player.on_save_pressed(move || {
        if let Some(app) = weak.upgrade() {
            c.save_pressed(&app);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    player.on_cycle_repeat(move || {
        if let Some(app) = weak.upgrade() {
            let p = app.global::<Player>();
            let next = match p.get_repeat().as_str() {
                "off" => "context",
                "context" => "track",
                _ => "off",
            };
            p.set_repeat(next.into());
            c.command("/player/repeat", json!({ "state": next }));
        }
    });
}

fn bind_shell(app: &App) {
    let shell = app.global::<Shell>();

    let weak = app.as_weak();
    shell.on_drag(move || {
        if let Some(app) = weak.upgrade() {
            app.window().with_winit_window(|w| {
                let _ = w.drag_window();
            });
        }
    });

    let weak = app.as_weak();
    shell.on_minimize(move || {
        if let Some(app) = weak.upgrade() {
            app.window().set_minimized(true);
        }
    });

    let weak = app.as_weak();
    shell.on_maximize(move || {
        if let Some(app) = weak.upgrade() {
            let maximized = !app.window().is_maximized();
            app.window().set_maximized(maximized);
            app.global::<Shell>().set_maximized(maximized);
        }
    });

    let weak = app.as_weak();
    shell.on_enter_mini(move || {
        if let Some(app) = weak.upgrade() {
            crate::window::enter_mini(&app);
        }
    });

    let weak = app.as_weak();
    shell.on_exit_mini(move || {
        if let Some(app) = weak.upgrade() {
            crate::window::exit_mini(&app);
        }
    });

    let weak = app.as_weak();
    shell.on_toggle_fullscreen(move || {
        if let Some(app) = weak.upgrade() {
            crate::window::toggle_fullscreen(&app);
        }
    });

    let weak = app.as_weak();
    shell.on_close(move || {
        if let Some(app) = weak.upgrade() {
            if app.global::<crate::Prefs>().get_minimize_on_close() {
                app.window().set_minimized(true);
                return;
            }
            crate::window::save(&app);
        }
        let _ = slint::quit_event_loop();
    });

    let weak = app.as_weak();
    shell.on_resize(move |dir| {
        let direction = match dir.as_str() {
            "n" => ResizeDirection::North,
            "s" => ResizeDirection::South,
            "e" => ResizeDirection::East,
            "w" => ResizeDirection::West,
            "ne" => ResizeDirection::NorthEast,
            "nw" => ResizeDirection::NorthWest,
            "se" => ResizeDirection::SouthEast,
            _ => ResizeDirection::SouthWest,
        };
        if let Some(app) = weak.upgrade() {
            app.window().with_winit_window(|w| {
                let _ = w.drag_resize_window(direction);
            });
        }
    });
}
