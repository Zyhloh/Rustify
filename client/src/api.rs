use crate::config::Config;
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

const KEY_HEADER: &str = "x-rustify-key";

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    key: Arc<RwLock<String>>,
}

fn read_key() -> String {
    std::fs::read_to_string(Config::data_dir().join("api.key"))
        .map(|k| k.trim().to_owned())
        .unwrap_or_default()
}

impl Api {
    pub fn new(base: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("http client");
        Self {
            http,
            base,
            key: Arc::new(RwLock::new(read_key())),
        }
    }

    pub fn key(&self) -> String {
        let cached = self.key.read().map(|k| k.clone()).unwrap_or_default();
        if !cached.is_empty() {
            return cached;
        }
        self.reload_key()
    }

    fn reload_key(&self) -> String {
        let key = read_key();
        if let Ok(mut slot) = self.key.write() {
            *slot = key.clone();
        }
        key
    }

    async fn send(&self, build: impl Fn() -> RequestBuilder) -> anyhow::Result<Response> {
        let res = build().header(KEY_HEADER, self.key()).send().await?;
        if res.status() != StatusCode::UNAUTHORIZED {
            return Ok(res.error_for_status()?);
        }
        let key = self.reload_key();
        Ok(build().header(KEY_HEADER, key).send().await?.error_for_status()?)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub async fn get_slow<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = self.url(path);
        Ok(self.send(|| self.http.get(&url).timeout(Duration::from_secs(120))).await?.json().await?)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = self.url(path);
        Ok(self.send(|| self.http.get(&url)).await?.json().await?)
    }

    pub async fn post_json<T: DeserializeOwned>(&self, path: &str, body: Value) -> anyhow::Result<T> {
        let url = self.url(path);
        Ok(self.send(|| self.http.post(&url).json(&body)).await?.json().await?)
    }

    pub async fn post(&self, path: &str, body: Value) -> anyhow::Result<()> {
        let url = self.url(path);
        self.send(|| self.http.post(&url).json(&body)).await?;
        Ok(())
    }
}
