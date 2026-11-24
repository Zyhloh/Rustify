use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub api: ApiConfig,
    pub spotify: SpotifyConfig,
    pub player: PlayerConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SpotifyConfig {
    pub client_id: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerConfig {
    pub device_name: String,
    pub initial_volume: u8,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn data_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Rustify")
    }
}
