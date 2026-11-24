#![cfg_attr(not(feature = "dev"), windows_subsystem = "windows")]

mod account;
mod api;
mod bindings;
mod config;
mod ctx;
mod find;
mod images;
mod items;
mod menu;
mod models;
mod news;
mod pages;
mod podcast;
mod prefs;
mod sidecar;
mod panels;
mod plugins;
mod search;
mod state;
mod tray;
mod window;

use config::Config;
use ctx::Ctx;
use slint::{ComponentHandle, Timer, TimerMode};
use std::time::Duration;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wgpu_core=error,wgpu_hal=error,naga=error".into()),
        )
        .init();

    let Some(instance) = tray::acquire() else { return Ok(()) };
    let start_hidden = std::env::args().any(|a| a == "--tray");

    let config = Config::load()?;
    std::thread::spawn(|| prefs::prune_image_cache(300 * 1024 * 1024));
    let ready = config.is_ready();
    if ready {
        sidecar::ensure_api(&config);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;

    let app = App::new()?;
    let ctx = Ctx::new(runtime.handle().clone(), config, app.as_weak());

    state::install(&app);
    pages::install(&app);
    menu::install(&app);
    panels::install(&app);
    account::install(&app);
    podcast::install(&app);
    find::install(&app);
    news::install(&app);
    prefs::load(&app);
    let recent_keys = search::install(&app);
    bindings::bind(&app, &ctx);
    plugins::install(&app, &ctx);
    ctx.load_images(recent_keys);
    if ready {
        ctx.spawn(ctx.clone().boot());
    } else {
        app.global::<Store>().set_needs_setup(true);
    }

    window::restore_when_ready(app.as_weak());

    let ticker = Timer::default();
    let weak = app.as_weak();
    ticker.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        if let Some(app) = weak.upgrade() {
            state::tick(&app);
        }
    });

    let _tray = tray::create();
    let tray_timer = Timer::default();
    let weak = app.as_weak();
    tray_timer.start(TimerMode::Repeated, Duration::from_millis(120), move || {
        if let Some(app) = weak.upgrade() {
            tray::poll(&app, &instance);
        }
    });

    if !start_hidden {
        app.show()?;
    }
    slint::run_event_loop_until_quit()?;
    window::save(&app);
    runtime.shutdown_timeout(Duration::from_millis(200));
    plugins::shutdown();
    sidecar::stop();
    Ok(())
}
