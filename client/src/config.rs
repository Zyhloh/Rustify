use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct ApiConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub api: ApiConfig,
    pub client_id: String,
    pub path: PathBuf,
}

const SCOPES: [&str; 16] = [
    "streaming",
    "user-read-private",
    "user-read-email",
    "user-read-playback-state",
    "user-modify-playback-state",
    "user-read-currently-playing",
    "user-library-read",
    "user-library-modify",
    "user-follow-read",
    "user-follow-modify",
    "user-top-read",
    "user-read-recently-played",
    "playlist-read-private",
    "playlist-read-collaborative",
    "playlist-modify-private",
    "playlist-modify-public",
];

fn default_config() -> Value {
    json!({
        "api": { "host": "127.0.0.1", "port": 7878 },
        "spotify": {
            "client_id": "",
            "redirect_uri": "http://127.0.0.1:8898/login",
            "scopes": SCOPES,
        },
        "player": { "device_name": "Rustify", "initial_volume": 70 }
    })
}

pub fn valid_client_id(id: &str) -> bool {
    id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())
}

impl Config {
    fn locate() -> PathBuf {
        let local = PathBuf::from("config.json");
        if local.exists() {
            return local.canonicalize().unwrap_or(local);
        }
        let beside = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join("config.json")));
        match beside {
            Some(path) if path.exists() => path,
            _ => Self::data_dir().join("config.json"),
        }
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::locate();
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, serde_json::to_string_pretty(&default_config())?)?;
        }
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        let api: ApiConfig = serde_json::from_value(raw.get("api").cloned().unwrap_or_else(|| default_config()["api"].clone()))?;
        let client_id = raw.pointer("/spotify/client_id").and_then(Value::as_str).unwrap_or_default().trim().to_owned();
        Ok(Self { api, client_id, path })
    }

    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new("."))
    }

    pub fn is_ready(&self) -> bool {
        valid_client_id(&self.client_id)
    }

    pub fn save_client_id(&mut self, id: &str) -> anyhow::Result<()> {
        let id = id.trim();
        if !valid_client_id(id) {
            anyhow::bail!("That doesn't look like a client ID. It should be 32 letters and numbers.");
        }
        let mut raw: Value = std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(default_config);
        if raw.get("spotify").is_none() {
            raw["spotify"] = default_config()["spotify"].clone();
        }
        raw["spotify"]["client_id"] = json!(id);
        std::fs::write(&self.path, serde_json::to_string_pretty(&raw)?)?;
        self.client_id = id.to_owned();
        Ok(())
    }

    pub fn http_url(&self) -> String {
        format!("http://{}:{}", self.api.host, self.api.port)
    }

    pub fn ws_url(&self) -> String {
        format!("ws://{}:{}/events", self.api.host, self.api.port)
    }

    pub fn data_dir() -> PathBuf {
        dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("Rustify")
    }
}
