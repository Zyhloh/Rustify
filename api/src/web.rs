use crate::auth::Auth;
use anyhow::anyhow;
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BASE: &str = "https://api.spotify.com/v1";
const MAX_WAIT: u64 = 30;

#[derive(Clone)]
pub struct WebApi {
    http: reqwest::Client,
    auth: Arc<Auth>,
    blocked: Arc<Mutex<HashMap<String, Instant>>>,
    lanes: Arc<tokio::sync::Semaphore>,
    pause: Arc<Mutex<Option<Instant>>>,
}

fn endpoint(method: &Method, path: &str) -> String {
    let route = path.split('?').next().unwrap_or(path);
    let shape: Vec<&str> = route
        .split('/')
        .map(|seg| {
            if seg.len() >= 16 && seg.chars().all(|c| c.is_ascii_alphanumeric()) {
                "{id}"
            } else {
                seg
            }
        })
        .collect();
    format!("{method} {}", shape.join("/"))
}

fn limits_path(client_id: &str) -> std::path::PathBuf {
    let tag: String = client_id.chars().filter(char::is_ascii_alphanumeric).take(32).collect();
    crate::config::Config::data_dir().join(format!("limits-{tag}.json"))
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn load_limits(client_id: &str) -> HashMap<String, Instant> {
    let stored: HashMap<String, u64> = std::fs::read(limits_path(client_id))
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_default();
    let now = unix_now();
    stored
        .into_iter()
        .filter(|(_, until)| *until > now)
        .filter_map(|(key, until)| Instant::now().checked_add(Duration::from_secs((until - now).min(48 * 3600))).map(|at| (key, at)))
        .collect()
}

fn save_limits(client_id: &str, blocked: &HashMap<String, Instant>) {
    let now = Instant::now();
    let base = unix_now();
    let stored: HashMap<&String, u64> = blocked
        .iter()
        .filter(|(_, until)| **until > now)
        .map(|(key, until)| (key, base + until.saturating_duration_since(now).as_secs()))
        .collect();
    if let Ok(raw) = serde_json::to_vec(&stored) {
        let _ = std::fs::write(limits_path(client_id), raw);
    }
}

impl WebApi {
    pub fn new(auth: Arc<Auth>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("http client");
        let blocked = Arc::new(Mutex::new(load_limits(auth.client_id())));
        Self {
            http,
            auth,
            blocked,
            lanes: Arc::new(tokio::sync::Semaphore::new(4)),
            pause: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let value = self.send(Method::GET, path, None).await?;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn post(&self, path: &str, body: Option<Value>) -> anyhow::Result<Value> {
        self.send(Method::POST, path, body).await
    }

    pub async fn put(&self, path: &str, body: Option<Value>) -> anyhow::Result<()> {
        self.send(Method::PUT, path, body).await.map(|_| ())
    }

    pub async fn delete(&self, path: &str, body: Option<Value>) -> anyhow::Result<()> {
        self.send(Method::DELETE, path, body).await.map(|_| ())
    }

    fn blocked_until(&self, key: &str) -> Option<Instant> {
        let mut blocked = self.blocked.lock().ok()?;
        match blocked.get(key) {
            Some(until) if *until > Instant::now() => Some(*until),
            Some(_) => {
                blocked.remove(key);
                None
            }
            None => None,
        }
    }

    fn hold_off(&self) -> Option<Duration> {
        let until = *self.pause.lock().ok()?;
        until.and_then(|u| u.checked_duration_since(Instant::now()))
    }

    fn block(&self, key: &str, wait: Duration) {
        let Some(until) = Instant::now().checked_add(wait) else { return };
        if let Ok(mut blocked) = self.blocked.lock() {
            blocked.insert(key.to_owned(), until);
            save_limits(self.auth.client_id(), &blocked);
        }
    }

    async fn send(&self, method: Method, path: &str, body: Option<Value>) -> anyhow::Result<Value> {
        let key = endpoint(&method, path);
        if let Some(until) = self.blocked_until(&key) {
            let mins = until.saturating_duration_since(Instant::now()).as_secs() / 60;
            return Err(anyhow!("{key} is rate limited by Spotify for another {mins} min"));
        }

        let url = if path.starts_with("http") {
            path.to_owned()
        } else {
            format!("{BASE}{path}")
        };

        let mut refreshed = false;
        for attempt in 0..3 {
            if let Some(wait) = self.hold_off() {
                tokio::time::sleep(wait).await;
            }
            let lane = self.lanes.acquire().await?;
            tracing::debug!(target: "webapi", "{key}");
            let token = self.auth.access_token().await?;
            let mut req = self.http.request(method.clone(), &url).bearer_auth(token);
            if let Some(body) = &body {
                req = req.json(body);
            }
            let res = req.send().await?;
            drop(lane);
            let status = res.status();

            if status == StatusCode::TOO_MANY_REQUESTS {
                let wait = res
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(2)
                    .clamp(1, 48 * 3600);
                if wait > MAX_WAIT || attempt == 2 {
                    tracing::warn!("{key} rate limited by Spotify for {} min, skipping until then", wait.div_ceil(60));
                    self.block(&key, Duration::from_secs(wait));
                    return Err(anyhow!("{key} is rate limited by Spotify for {} min", wait.div_ceil(60)));
                }
                if let Ok(mut pause) = self.pause.lock() {
                    *pause = Instant::now().checked_add(Duration::from_secs(wait));
                }
                continue;
            }
            if status == StatusCode::UNAUTHORIZED && !refreshed {
                refreshed = true;
                self.auth.forget_live().await;
                continue;
            }
            if !status.is_success() {
                let text = res.text().await.unwrap_or_default();
                return Err(anyhow!("{method} {path} -> {status}: {text}"));
            }
            let bytes = res.bytes().await?;
            if bytes.is_empty() {
                return Ok(Value::Null);
            }
            return Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null));
        }
        Err(anyhow!("{method} {path} -> rate limited"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn serve(replies: Vec<(u16, &'static str, String)>) -> (String, Arc<Mutex<Vec<String>>>, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let (log, count) = (seen.clone(), hits.clone());
        tokio::spawn(async move {
            for (status, extra, body) in replies {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                let mut buf = vec![0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let auth = request.lines().find(|l| l.to_ascii_lowercase().starts_with("authorization:")).unwrap_or_default().to_owned();
                log.lock().unwrap().push(auth);
                count.fetch_add(1, Ordering::SeqCst);
                let reply = format!("HTTP/1.1 {status} X
content-length: {}
content-type: application/json
connection: close
{extra}
{body}", body.len());
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        (url, seen, hits)
    }

    #[tokio::test]
    async fn retries_after_short_rate_limit() {
        let (url, _, hits) = serve(vec![
            (429, "retry-after: 1
", String::new()),
            (200, "", r#"{"ok":true}"#.into()),
        ])
        .await;
        let api = WebApi::new(Arc::new(Auth::for_test(&["one"])));
        let started = Instant::now();
        let v: Value = api.get(&format!("{url}/v1/me")).await.unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(started.elapsed() >= Duration::from_millis(900));
    }

    #[tokio::test]
    async fn refreshes_token_after_unauthorized() {
        let (url, seen, hits) = serve(vec![
            (401, "", r#"{"error":"expired"}"#.into()),
            (200, "", r#"{"ok":true}"#.into()),
        ])
        .await;
        let api = WebApi::new(Arc::new(Auth::for_test(&["stale", "fresh"])));
        let v: Value = api.get(&format!("{url}/v1/me")).await.unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        let auth = seen.lock().unwrap().clone();
        assert!(auth[0].ends_with("stale"));
        assert!(auth[1].ends_with("fresh"));
    }

    #[tokio::test]
    async fn gives_up_cleanly_on_repeated_unauthorized() {
        let (url, _, hits) = serve(vec![
            (401, "", String::new()),
            (401, "", String::new()),
            (401, "", String::new()),
        ])
        .await;
        let api = WebApi::new(Arc::new(Auth::for_test(&["a", "b", "c"])));
        let result = api.get::<Value>(&format!("{url}/v1/me")).await;
        assert!(result.is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn rate_limit_pauses_other_requests() {
        let (url, _, hits) = serve(vec![
            (429, "retry-after: 2
", String::new()),
            (200, "", r#"{"a":1}"#.into()),
            (200, "", r#"{"b":1}"#.into()),
        ])
        .await;
        let api = WebApi::new(Arc::new(Auth::for_test(&["t"])));
        let first = api.clone();
        let u1 = format!("{url}/v1/one");
        let started = Instant::now();
        let a = tokio::spawn(async move { first.get::<Value>(&u1).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        let b: Value = api.get(&format!("{url}/v1/two")).await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(1800));
        assert!(a.await.unwrap().is_ok());
        assert!(b.is_object());
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }
}
