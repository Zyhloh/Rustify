use crate::config::Config;
use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::Response,
};
use std::{path::PathBuf, sync::Arc};

pub const HEADER: &str = "x-rustify-key";
const OPEN: [&str; 2] = ["/health", "/webplayer"];

#[derive(Clone)]
pub struct Guard {
    key: Arc<str>,
    host: Arc<str>,
}

pub fn key_path() -> PathBuf {
    Config::data_dir().join("api.key")
}

impl Guard {
    pub fn create(config: &Config) -> anyhow::Result<Self> {
        let key = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        std::fs::create_dir_all(Config::data_dir())?;
        std::fs::write(key_path(), &key)?;
        Ok(Self {
            key: key.into(),
            host: format!("{}:{}", config.api.host, config.api.port).into(),
        })
    }

    fn host_ok(&self, request: &Request) -> bool {
        request
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| h == &*self.host || h == format!("localhost:{}", self.port()))
    }

    fn port(&self) -> &str {
        self.host.rsplit(':').next().unwrap_or_default()
    }

    fn key_ok(&self, request: &Request) -> bool {
        let from_header = request.headers().get(HEADER).and_then(|v| v.to_str().ok());
        let from_query = (request.uri().path() == "/events")
            .then(|| request.uri().query())
            .flatten()
            .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("key=")));
        from_header.or(from_query).is_some_and(|k| constant_eq(k.as_bytes(), self.key.as_bytes()))
    }
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn check(State(guard): State<Guard>, request: Request, next: Next) -> Result<Response, StatusCode> {
    if !guard.host_ok(&request) {
        return Err(StatusCode::MISDIRECTED_REQUEST);
    }
    if OPEN.contains(&request.uri().path()) || guard.key_ok(&request) {
        return Ok(next.run(request).await);
    }
    Err(StatusCode::UNAUTHORIZED)
}
