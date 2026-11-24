use crate::config::{Config, SpotifyConfig};
use anyhow::{Context, anyhow};
use crate::oauth::{self, Token};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Instant};
use tokio::sync::Mutex;

#[derive(Serialize, Deserialize)]
struct StoredToken {
    refresh_token: String,
}

struct Live {
    access_token: String,
    expires_at: Instant,
}

pub struct Auth {
    client_id: String,
    redirect_uri: String,
    scopes: Vec<String>,
    path: PathBuf,
    refresh_token: Mutex<Option<String>>,
    live: Mutex<Option<Live>>,
    refreshing: Mutex<()>,
    #[cfg(test)]
    fake: std::sync::Mutex<Vec<String>>,
}

fn read_stored(dir: &std::path::Path) -> Option<String> {
    let sealed = dir.join("token.bin");
    if let Some(token) = std::fs::read(&sealed)
        .ok()
        .and_then(|raw| crate::vault::open(&raw))
        .and_then(|raw| serde_json::from_slice::<StoredToken>(&raw).ok())
    {
        return Some(token.refresh_token);
    }
    let legacy = dir.join("token.json");
    let token = std::fs::read_to_string(&legacy)
        .ok()
        .and_then(|raw| serde_json::from_str::<StoredToken>(&raw).ok())?;
    if let Some(sealed_bytes) = serde_json::to_vec(&token).ok().and_then(|raw| crate::vault::seal(&raw))
        && std::fs::write(&sealed, sealed_bytes).is_ok() {
            let _ = std::fs::remove_file(&legacy);
        }
    Some(token.refresh_token)
}

impl Auth {
    pub fn new(config: &SpotifyConfig) -> anyhow::Result<Self> {
        if config.client_id.is_empty() {
            return Err(anyhow!("spotify.client_id is empty in config.json"));
        }
        let dir = Config::data_dir();
        let refresh_token = read_stored(&dir);

        Ok(Self {
            client_id: config.client_id.clone(),
            redirect_uri: config.redirect_uri.clone(),
            scopes: config.scopes.clone(),
            path: dir.join("token.bin"),
            refresh_token: Mutex::new(refresh_token),
            live: Mutex::new(None),
            refreshing: Mutex::new(()),
            #[cfg(test)]
            fake: std::sync::Mutex::new(Vec::new()),
        })
    }

    #[cfg(test)]
    pub fn for_test(tokens: &[&str]) -> Self {
        Self {
            client_id: "test".into(),
            redirect_uri: String::new(),
            scopes: Vec::new(),
            path: std::env::temp_dir().join("rustify-test-token.bin"),
            refresh_token: Mutex::new(None),
            live: Mutex::new(None),
            refreshing: Mutex::new(()),
            fake: std::sync::Mutex::new(tokens.iter().rev().map(|t| t.to_string()).collect()),
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub async fn has_session(&self) -> bool {
        self.refresh_token.lock().await.is_some()
    }

    pub async fn login(&self) -> anyhow::Result<String> {
        let scopes: Vec<&str> = self.scopes.iter().map(String::as_str).collect();
        let token = oauth::authorize(&self.client_id, &self.redirect_uri, &scopes)
            .await
            .context("browser login failed")?;
        self.store(token).await
    }

    async fn cached(&self) -> Option<String> {
        self.live
            .lock()
            .await
            .as_ref()
            .filter(|live| live.expires_at > Instant::now() + std::time::Duration::from_secs(60))
            .map(|live| live.access_token.clone())
    }

    pub async fn access_token(&self) -> anyhow::Result<String> {
        if let Some(token) = self.cached().await {
            return Ok(token);
        }
        #[cfg(test)]
        if let Some(token) = self.fake.lock().ok().and_then(|mut f| f.pop()) {
            *self.live.lock().await = Some(Live { access_token: token.clone(), expires_at: Instant::now() + std::time::Duration::from_secs(3600) });
            return Ok(token);
        }
        let _guard = self.refreshing.lock().await;
        if let Some(token) = self.cached().await {
            return Ok(token);
        }
        let refresh = self
            .refresh_token
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow!("not logged in"))?;
        let token = match oauth::refresh(&self.client_id, &refresh).await {
            Ok(token) => token,
            Err(e) if e.downcast_ref::<oauth::Rejected>().is_some() && { let m = format!("{e:#}"); m.contains("invalid_grant") || m.contains("invalid_client") } => {
                tracing::warn!("saved login was rejected by Spotify, signing out: {e:#}");
                self.logout().await;
                return Err(anyhow!("not logged in"));
            }
            Err(e) => return Err(e.context("token refresh failed")),
        };
        self.store(token).await
    }

    pub async fn forget_live(&self) {
        *self.live.lock().await = None;
    }

    pub async fn logout(&self) {
        *self.refresh_token.lock().await = None;
        *self.live.lock().await = None;
        let _ = std::fs::remove_file(&self.path);
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::remove_file(dir.join("token.json"));
        }
    }

    async fn store(&self, token: Token) -> anyhow::Result<String> {
        if !token.refresh_token.is_empty() {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let stored = StoredToken {
                refresh_token: token.refresh_token.clone(),
            };
            let sealed = crate::vault::seal(&serde_json::to_vec(&stored)?).ok_or_else(|| anyhow!("could not protect the login token"))?;
            std::fs::write(&self.path, sealed)?;
            *self.refresh_token.lock().await = Some(token.refresh_token);
        }
        *self.live.lock().await = Some(Live {
            access_token: token.access_token.clone(),
            expires_at: token.expires_at,
        });
        Ok(token.access_token)
    }
}
