use anyhow::{Context, anyhow, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use url::Url;

const AUTHORIZE: &str = "https://accounts.spotify.com/authorize";
const TOKEN: &str = "https://accounts.spotify.com/api/token";
const WAIT: Duration = Duration::from_secs(300);

#[derive(Debug)]
pub struct Rejected;

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("saved login was rejected")
    }
}

impl std::error::Error for Rejected {}

pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Instant,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
}

impl From<TokenResponse> for Token {
    fn from(r: TokenResponse) -> Self {
        Self {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            expires_at: Instant::now() + Duration::from_secs(if r.expires_in == 0 { 3600 } else { r.expires_in }),
        }
    }
}

fn random() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn page(title: &str) -> String {
    format!(
        "<html><head><meta charset=\"utf-8\"><title>Rustify</title></head><body style=\"background:#121212;color:#fff;font-family:sans-serif;display:flex;align-items:center;justify-content:center;height:100vh;margin:0\"><h2>{title}</h2></body></html>"
    )
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    let launched = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let launched = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let launched = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(e) = launched {
        tracing::warn!("could not open the browser: {e}");
    }
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes()).await;
}

async fn request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = vec![0u8; 8192];
    let mut len = 0;
    while len < buf.len() {
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf[len..])).await.ok()?.ok()?;
        if read == 0 {
            break;
        }
        len += read;
        if buf[..len].windows(2).any(|w| w == b"\r\n") {
            break;
        }
    }
    let head = String::from_utf8_lossy(&buf[..len]);
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    (parts.next()? == "GET").then_some(())?;
    parts.next().map(str::to_owned)
}

async fn wait_for_code(listener: TcpListener, path: &str, state: &str) -> anyhow::Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Some(target) = request_target(&mut stream).await else { continue };
        let Ok(url) = Url::parse(&format!("http://localhost{target}")) else {
            respond(&mut stream, "400 Bad Request", &page("Bad request")).await;
            continue;
        };
        if url.path() != path {
            respond(&mut stream, "404 Not Found", "").await;
            continue;
        }
        let query = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
        if query("state").as_deref() != Some(state) {
            respond(&mut stream, "400 Bad Request", &page("This login link has expired. Start again from Rustify.")).await;
            continue;
        }
        if let Some(error) = query("error") {
            respond(&mut stream, "200 OK", &page("Login was cancelled. You can close this tab.")).await;
            bail!("login was cancelled ({error})");
        }
        if let Some(code) = query("code") {
            respond(&mut stream, "200 OK", &page("Logged in to Rustify. You can close this tab.")).await;
            return Ok(code);
        }
        respond(&mut stream, "400 Bad Request", &page("Missing login code")).await;
    }
}

pub async fn authorize(client_id: &str, redirect_uri: &str, scopes: &[&str]) -> anyhow::Result<Token> {
    let redirect = Url::parse(redirect_uri).context("redirect_uri is not a valid URL")?;
    let host = redirect.host_str().unwrap_or("127.0.0.1");
    let port = redirect.port().ok_or_else(|| anyhow!("redirect_uri needs a port"))?;
    let bind_host = if host == "localhost" { "127.0.0.1" } else { host };
    let listener = TcpListener::bind((bind_host, port)).await.with_context(|| format!("could not listen on {bind_host}:{port}"))?;

    let verifier = random();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random();
    let mut url = Url::parse(AUTHORIZE)?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &challenge)
        .append_pair("state", &state)
        .append_pair("scope", &scopes.join(" "));
    open_browser(url.as_str());

    let code = tokio::time::timeout(WAIT, wait_for_code(listener, redirect.path(), &state))
        .await
        .map_err(|_| anyhow!("login was not approved in time"))??;

    let response = reqwest::Client::new()
        .post(TOKEN)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", &verifier),
        ])
        .send()
        .await?
        .error_for_status()
        .context("Spotify rejected the login")?;
    Ok(response.json::<TokenResponse>().await?.into())
}

pub async fn refresh(client_id: &str, refresh_token: &str) -> anyhow::Result<Token> {
    let response = reqwest::Client::new()
        .post(TOKEN)
        .form(&[("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", client_id)])
        .send()
        .await?;
    if matches!(response.status().as_u16(), 400 | 401) {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::Error::new(Rejected).context(body));
    }
    let response = response.error_for_status().context("Spotify rejected the saved login")?;
    let mut token: Token = response.json::<TokenResponse>().await?.into();
    if token.refresh_token.is_empty() {
        token.refresh_token = refresh_token.to_owned();
    }
    Ok(token)
}
