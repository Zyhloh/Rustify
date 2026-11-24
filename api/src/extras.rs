use crate::collection::{TrackRow, format_duration, track_row};
use crate::models::{ArtistRef, Card, Me, image, s};
use crate::web::WebApi;
use librespot::core::session::Session;
use reqwest::Method;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize)]
pub struct Queue {
    pub now: Option<TrackRow>,
    pub next: Vec<TrackRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub active: bool,
    pub current: bool,
    pub volume: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LyricLine {
    pub start_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Lyrics {
    pub synced: bool,
    pub lines: Vec<LyricLine>,
    pub background: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Friend {
    pub user_uri: String,
    pub name: String,
    pub image: Option<String>,
    pub track_uri: String,
    pub track: String,
    pub artist: String,
    pub artist_uri: String,
    pub context: String,
    pub context_uri: String,
    pub minutes_ago: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    pub followers: u64,
    pub following: u64,
    pub is_me: bool,
    pub playlists: Vec<Card>,
    pub top_artists: Vec<Card>,
    pub top_tracks: Vec<TrackRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EpisodeRow {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub description: String,
    pub date: String,
    pub duration: String,
    pub image: Option<String>,
    pub show: String,
    pub show_id: String,
    pub explicit: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShowPage {
    pub id: String,
    pub uri: String,
    pub title: String,
    pub publisher: String,
    pub description: String,
    pub image: Option<String>,
    pub saved: bool,
    pub episodes: Vec<EpisodeRow>,
}

pub async fn native_me(session: &Session) -> anyhow::Result<Me> {
    let id = session.username();
    let path = format!("/user-profile-view/v3/profile/{id}?playlist_limit=0&artist_limit=0&episode_limit=0");
    let raw = session.spclient().request_as_json(&Method::GET, &path, None, None).await?;
    let view: Value = serde_json::from_slice(&raw)?;
    let name = s(&view, "name");
    Ok(Me {
        display_name: if name.is_empty() { id.clone() } else { name },
        image: spotify_image(&s(&view, "image_url")),
        product: session.get_user_attribute("type").unwrap_or_else(|| "premium".into()),
        email: String::new(),
        country: session.country(),
        followers: view.get("followers_count").and_then(Value::as_u64).unwrap_or(0),
        id,
    })
}

pub fn spotify_image(raw: &str) -> Option<String> {
    if raw.is_empty() {
        None
    } else if let Some(id) = raw.strip_prefix("spotify:image:") {
        Some(format!("https://i.scdn.co/image/{id}"))
    } else if let Some(ids) = raw.strip_prefix("spotify:mosaic:") {
        Some(format!("https://mosaic.scdn.co/640/{}", ids.replace(':', "")))
    } else {
        Some(raw.to_owned())
    }
}

fn episode_row(v: &Value) -> Option<EpisodeRow> {
    let id = s(v, "id");
    if id.is_empty() {
        return None;
    }
    let show = v.get("show").cloned().unwrap_or_default();
    Some(EpisodeRow {
        id,
        uri: s(v, "uri"),
        name: s(v, "name"),
        description: s(v, "description"),
        date: s(v, "release_date"),
        duration: format_duration(v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0)),
        image: image(v, 160).or_else(|| image(&show, 160)),
        show: s(&show, "name"),
        show_id: s(&show, "id"),
        explicit: v.get("explicit").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn episode_as_track(v: &Value) -> Option<TrackRow> {
    let e = episode_row(v)?;
    Some(TrackRow {
        id: e.id,
        uri: e.uri,
        album_id: String::new(),
        name: e.name,
        artists: vec![ArtistRef { id: String::new(), name: e.show.clone() }],
        album: e.show,
        image: e.image,
        duration: e.duration,
        explicit: e.explicit,
        added: String::new(),
        added_by: String::new(),
    })
}

fn any_row(v: &Value) -> Option<TrackRow> {
    match s(v, "type").as_str() {
        "episode" => episode_as_track(v),
        _ => track_row(v, "", String::new()),
    }
}

pub async fn queue(api: &WebApi) -> anyhow::Result<Queue> {
    let v: Value = api.get("/me/player/queue").await?;
    Ok(Queue {
        now: v.get("currently_playing").filter(|c| !c.is_null()).and_then(any_row),
        next: v
            .get("queue")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(any_row)
            .collect(),
    })
}

pub async fn devices(api: &WebApi) -> anyhow::Result<Vec<Device>> {
    let v: Value = api.get("/me/player/devices").await?;
    Ok(v.get("devices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|d| Device {
            id: s(d, "id"),
            name: s(d, "name"),
            kind: s(d, "type"),
            active: d.get("is_active").and_then(Value::as_bool).unwrap_or(false),
            current: false,
            volume: d.get("volume_percent").and_then(Value::as_u64),
        })
        .collect())
}

pub async fn transfer(api: &WebApi, device: &str, play: bool) -> anyhow::Result<()> {
    api.put("/me/player", Some(json!({ "device_ids": [device], "play": play }))).await
}

fn argb_hex(value: Option<&Value>) -> String {
    let raw = value.and_then(Value::as_i64).unwrap_or(-12_566_464) as u32;
    format!("#{:06x}", raw & 0x00ff_ffff)
}

pub async fn lyrics(session: &Session, track_id: &str) -> anyhow::Result<Lyrics> {
    let path = format!("/color-lyrics/v2/track/{track_id}?format=json&vocalRemoval=false&market=from_token");
    let bytes = session
        .spclient()
        .request_as_json(&Method::GET, &path, None, None)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let v: Value = serde_json::from_slice(&bytes)?;
    let body = v.get("lyrics").cloned().unwrap_or_default();
    let synced = s(&body, "syncType") == "LINE_SYNCED";
    Ok(Lyrics {
        synced,
        lines: body
            .get("lines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|l| LyricLine {
                start_ms: s(l, "startTimeMs").parse().unwrap_or(0),
                text: s(l, "words"),
            })
            .collect(),
        background: argb_hex(v.pointer("/colors/background")),
        provider: s(&body, "providerDisplayName"),
    })
}

pub async fn friends(session: &Session) -> anyhow::Result<Vec<Friend>> {
    let bytes = session
        .spclient()
        .request_as_json(&Method::GET, "/presence-view/v1/buddylist", None, None)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let v: Value = serde_json::from_slice(&bytes)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Ok(v.get("friends")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| {
            let user = f.get("user").cloned().unwrap_or_default();
            let track = f.get("track").cloned().unwrap_or_default();
            let artist = track.get("artist").cloned().unwrap_or_default();
            let context = track.get("context").cloned().unwrap_or_default();
            let at = f.get("timestamp").and_then(Value::as_u64).unwrap_or(now_ms);
            Friend {
                user_uri: s(&user, "uri"),
                name: s(&user, "name"),
                image: spotify_image(&s(&user, "imageUrl")),
                track_uri: s(&track, "uri"),
                track: s(&track, "name"),
                artist: s(&artist, "name"),
                artist_uri: s(&artist, "uri"),
                context: s(&context, "name"),
                context_uri: s(&context, "uri"),
                minutes_ago: now_ms.saturating_sub(at) / 60_000,
            }
        })
        .collect())
}

pub async fn profile(api: &WebApi, session: Option<&Session>, me: &Me, user: &str) -> anyhow::Result<Profile> {
    let id = if user == "me" { me.id.clone() } else { user.to_owned() };
    let is_me = id == me.id;

    let view = match session {
        Some(session) => {
            let path = format!(
                "/user-profile-view/v3/profile/{id}?playlist_limit=50&artist_limit=20&episode_limit=0&market=from_token"
            );
            session
                .spclient()
                .request_as_json(&Method::GET, &path, None, None)
                .await
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or_default()
        }
        None => Value::Null,
    };

    let playlists = view
        .get("public_playlists")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|p| {
            let uri = s(p, "uri");
            Card {
                id: uri.rsplit(':').next().unwrap_or_default().to_owned(),
                uri,
                kind: "playlist".into(),
                title: s(p, "name"),
                subtitle: format!("By {}", s(p, "owner_name")),
                image: spotify_image(&s(p, "image_url")),
                pinned: false,
            }
        })
        .collect();

    let (top_artists, top_tracks) = if is_me {
        let (artists, tracks) = tokio::join!(
            api.get::<Value>("/me/top/artists?limit=10&time_range=short_term"),
            api.get::<Value>("/me/top/tracks?limit=10&time_range=short_term"),
        );
        let list = |v: anyhow::Result<Value>| -> Vec<Value> {
            v.ok().and_then(|v| v.get("items").and_then(Value::as_array).cloned()).unwrap_or_default()
        };
        (
            list(artists).iter().map(Card::artist).collect(),
            list(tracks).iter().filter_map(|t| track_row(t, "", String::new())).collect(),
        )
    } else {
        (Vec::new(), Vec::new())
    };

    let name = s(&view, "name");
    Ok(Profile {
        id,
        name: if name.is_empty() && is_me { me.display_name.clone() } else { name },
        image: spotify_image(&s(&view, "image_url")).or_else(|| if is_me { me.image.clone() } else { None }),
        followers: view.get("followers_count").and_then(Value::as_u64).unwrap_or(0),
        following: view.get("following_count").and_then(Value::as_u64).unwrap_or(0),
        is_me,
        playlists,
        top_artists,
        top_tracks,
    })
}

pub async fn show(api: &WebApi, id: &str) -> anyhow::Result<ShowPage> {
    let v: Value = api.get(&format!("/shows/{id}")).await?;
    Ok(ShowPage {
        id: s(&v, "id"),
        uri: s(&v, "uri"),
        title: s(&v, "name"),
        publisher: s(&v, "publisher"),
        description: s(&v, "description"),
        image: image(&v, 640),
        saved: false,
        episodes: v
            .pointer("/episodes/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let mut row = episode_row(e)?;
                if row.show.is_empty() {
                    row.show = s(&v, "name");
                    row.show_id = s(&v, "id");
                }
                Some(row)
            })
            .collect(),
    })
}

pub async fn saved_episodes(api: &WebApi) -> anyhow::Result<ShowPage> {
    let v: Value = api.get("/me/episodes?limit=50").await?;
    Ok(ShowPage {
        id: "episodes".into(),
        uri: "spotify:collection:your-episodes".into(),
        title: "Your Episodes".into(),
        publisher: String::new(),
        description: String::new(),
        image: None,
        saved: true,
        episodes: v
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("episode").and_then(episode_row))
            .collect(),
    })
}

pub async fn edit_playlist(api: &WebApi, id: &str, name: &str, description: &str) -> anyhow::Result<()> {
    api.put(&format!("/playlists/{id}"), Some(json!({ "name": name, "description": description }))).await
}

pub async fn unfollow_playlist(api: &WebApi, id: &str) -> anyhow::Result<()> {
    api.delete(&format!("/playlists/{id}/followers"), None).await
}

pub async fn recent(api: &WebApi) -> anyhow::Result<Vec<TrackRow>> {
    let v: Value = api.get("/me/player/recently-played?limit=50").await?;
    let mut seen = std::collections::HashSet::new();
    Ok(v.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("track"))
        .filter(|t| seen.insert(s(t, "id")))
        .filter_map(|t| track_row(t, "", String::new()))
        .collect())
}
