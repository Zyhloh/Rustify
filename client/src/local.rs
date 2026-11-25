use crate::{App, LocalFiles, ctx::Ctx};
use serde::Deserialize;
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;

#[derive(Debug, Clone, Default, Deserialize)]
struct Settings {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    folders: Vec<String>,
    #[serde(default)]
    count: usize,
}

fn show(app: &App, settings: &Settings) {
    let local = app.global::<LocalFiles>();
    local.set_enabled(settings.enabled);
    local.set_folders(ModelRc::from(Rc::new(VecModel::from(settings.folders.iter().map(SharedString::from).collect::<Vec<_>>()))));
    local.set_count(settings.count as i32);
}

pub fn install(app: &App, ctx: &Ctx) {
    let local = app.global::<LocalFiles>();

    let (c, weak) = (ctx.clone(), app.as_weak());
    local.on_set_enabled(move |on| {
        if let Some(app) = weak.upgrade() {
            let mut folders = current_folders(&app);
            if on && folders.is_empty() {
                folders = [dirs::audio_dir(), dirs::download_dir()].into_iter().flatten().map(|p| p.display().to_string()).collect();
            }
            save(&c, &app, on, folders);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    local.on_add_folder(move || {
        let (c, weak) = (c.clone(), weak.clone());
        let _ = std::thread::Builder::new().name("rustify-folder-picker".into()).spawn(move || {
            let picked = rfd::FileDialog::new().set_title("Add a folder with music").pick_folder();
            if let Some(path) = picked {
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak.upgrade() {
                        let mut folders = current_folders(&app);
                        let path = path.display().to_string();
                        if !folders.contains(&path) {
                            folders.push(path);
                        }
                        save(&c, &app, true, folders);
                    }
                });
            }
        });
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    local.on_remove_folder(move |folder| {
        if let Some(app) = weak.upgrade() {
            let folders: Vec<String> = current_folders(&app).into_iter().filter(|f| f.as_str() != folder.as_str()).collect();
            let enabled = app.global::<LocalFiles>().get_enabled();
            save(&c, &app, enabled, folders);
        }
    });

    let (c, weak) = (ctx.clone(), app.as_weak());
    local.on_rescan(move || {
        if let Some(app) = weak.upgrade() {
            app.global::<LocalFiles>().set_scanning(true);
            let ctx = c.clone();
            c.spawn(async move {
                let result = ctx.api.post_json::<Value>("/local/rescan", json!({})).await;
                ctx.reload_library().await;
                ctx.ui(move |app| {
                    let local = app.global::<LocalFiles>();
                    local.set_scanning(false);
                    if let Ok(v) = result {
                        local.set_count(v.get("count").and_then(Value::as_u64).unwrap_or(0) as i32);
                    }
                });
            });
        }
    });

    let c = ctx.clone();
    ctx.spawn(async move {
        if let Ok(settings) = c.api.get::<Settings>("/local/settings").await {
            c.ui(move |app| show(app, &settings));
        }
    });
}

fn current_folders(app: &App) -> Vec<String> {
    use slint::Model;
    app.global::<LocalFiles>().get_folders().iter().map(|f| f.to_string()).collect()
}

fn save(ctx: &Ctx, app: &App, enabled: bool, folders: Vec<String>) {
    let local = app.global::<LocalFiles>();
    show(app, &Settings { enabled, folders: folders.clone(), count: local.get_count() as usize });
    local.set_scanning(true);
    let c = ctx.clone();
    ctx.spawn(async move {
        let result = c.api.post_json::<Value>("/local/settings", json!({ "enabled": enabled, "folders": folders })).await;
        c.reload_library().await;
        let settings = c.api.get::<Settings>("/local/settings").await;
        c.ui(move |app| {
            app.global::<LocalFiles>().set_scanning(false);
            match (result, settings) {
                (Ok(_), Ok(settings)) => show(app, &settings),
                _ => crate::menu::toast(app, "Couldn't save your local files settings"),
            }
        });
    });
}
