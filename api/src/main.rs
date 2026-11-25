mod artist;
mod auth;
mod collection;
mod config;
mod extras;
mod guard;
mod home;
mod ids;
mod local;
mod models;
mod native;
mod oauth;
mod player;
mod routes;
mod vault;
mod search;
mod web;
mod webplayer;

use auth::Auth;
use config::Config;
use routes::AppState;
use std::{net::SocketAddr, sync::Arc};

fn init_logging() {
    use tracing_subscriber::{EnvFilter, Layer, fmt, layer::SubscriberExt, util::SubscriberInitExt};

    let console = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,librespot=warn".into());
    let _ = std::fs::create_dir_all(Config::data_dir());
    let file = std::fs::File::create(Config::data_dir().join("api.log")).ok();

    tracing_subscriber::registry()
        .with(fmt::layer().with_filter(console))
        .with(file.map(|f| {
            fmt::layer()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(f))
                .with_filter(EnvFilter::new("info,librespot=warn"))
        }))
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();
    let _ = rustls::crypto::ring::default_provider().install_default();
    std::panic::set_hook(Box::new(|info| tracing::error!("panic: {info}")));

    let config = Config::load("config.json")?;
    let addr: SocketAddr = format!("{}:{}", config.api.host, config.api.port).parse()?;
    let auth = Arc::new(Auth::new(&config.spotify)?);
    let guard = guard::Guard::create(&config)?;
    let state = AppState::new(config, auth);
    let local = state.local.clone();
    tokio::task::spawn_blocking(move || local.scan());

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("api listening on {addr}");
    tokio::spawn(state.clone().boot(false));

    let app = routes::router(state).layer(axum::middleware::from_fn_with_state(guard, guard::check));
    axum::serve(listener, app).await?;
    Ok(())
}
