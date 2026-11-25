use crate::models::{ArtistRef, Me, image, s};
use crate::web::WebApi;
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::RwLock;

const PAGE: u64 = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRow {
    pub id: String,
    pub uri: String,
    pub album_id: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub album: String,
    pub image: Option<String>,
    pub duration: String,
    pub explicit: bool,
    pub added: String,
    pub added_by: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub local_file: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Collection {
    pub id: String,
    pub uri: String,
    pub context: String,
    pub kind: String,
    pub label: String,
    pub title: String,
    pub description: String,
    pub owner: String,
    pub owner_is_me: bool,
    pub image: Option<String>,
    pub meta: String,
    pub show_album: bool,
    pub show_added: bool,
    pub show_added_by: bool,
    pub show_art: bool,
    pub tracks: Vec<TrackRow>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub owner_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_image: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub footer: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub more: Vec<crate::models::Card>,
    pub partial: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaylistOption {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedState {
    pub liked: bool,
    pub playlists: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Indexed {
    snapshot: String,
    tracks: HashSet<String>,
}

fn index_path() -> std::path::PathBuf {
    crate::config::Config::data_dir().join("index.json")
}

#[derive(Default)]
pub struct PlaylistIndex {
    playlists: RwLock<Vec<PlaylistOption>>,
    entries: RwLock<HashMap<String, Indexed>>,
}

fn items(v: &Value) -> Vec<Value> {
    v.get("items").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn total(v: &Value) -> u64 {
    v.pointer("/items/total")
        .or_else(|| v.pointer("/tracks/total"))
        .or_else(|| v.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

pub fn format_duration(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn format_total(ms: u64) -> String {
    let secs = ms / 1000;
    match (secs / 3600, secs % 3600 / 60, secs % 60) {
        (0, 0, s) => format!("{s} sec"),
        (0, m, s) => format!("{m} min {s} sec"),
        (h, m, _) => format!("{h} hr {m} min"),
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn relative_date(iso: &str) -> String {
    let parts: Vec<i64> = iso
        .get(..10)
        .unwrap_or_default()
        .split('-')
        .filter_map(|p| p.parse().ok())
        .collect();
    let [y, m, d] = parts[..] else { return String::new() };
    let today = (SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) / 86_400) as i64;
    let days = today - days_from_civil(y, m, d);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    match days {
        ..=0 => "Today".into(),
        1 => "1 day ago".into(),
        2..=6 => format!("{days} days ago"),
        7..=13 => "1 week ago".into(),
        14..=27 => format!("{} weeks ago", days / 7),
        _ => format!("{} {d}, {y}", MONTHS[(m.clamp(1, 12) - 1) as usize]),
    }
}

pub fn artists(v: &Value) -> Vec<ArtistRef> {
    v.get("artists")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|x| ArtistRef { id: s(x, "id"), name: s(x, "name") }).collect())
        .unwrap_or_default()
}

pub fn track_row(track: &Value, added_at: &str, added_by: String) -> Option<TrackRow> {
    let id = s(track, "id");
    let uri = s(track, "uri");
    if id.is_empty() && uri.starts_with("spotify:local:") {
        let local = crate::local::parse_uri(&uri)?;
        let name = Some(s(track, "name")).filter(|n| !n.is_empty()).unwrap_or(local.title);
        let mut artists = artists(track);
        if artists.iter().all(|a| a.name.is_empty()) {
            artists = if local.artist.is_empty() { Vec::new() } else { vec![ArtistRef { id: String::new(), name: local.artist }] };
        }
        let album = track.get("album").map(|a| s(a, "name")).filter(|n| !n.is_empty()).unwrap_or(local.album);
        let ms = track.get("duration_ms").and_then(Value::as_u64).filter(|d| *d > 0).unwrap_or(local.seconds as u64 * 1000);
        return Some(TrackRow {
            id: String::new(),
            uri,
            album_id: String::new(),
            name,
            artists,
            album,
            image: None,
            duration: format_duration(ms),
            explicit: false,
            added: relative_date(added_at),
            added_by,
            local_file: false,
        });
    }
    if id.is_empty() {
        return None;
    }
    let album = track.get("album").cloned().unwrap_or_default();
    Some(TrackRow {
        id,
        uri: s(track, "uri"),
        album_id: s(&album, "id"),
        name: s(track, "name"),
        artists: artists(track),
        album: s(&album, "name"),
        image: image(&album, 64),
        duration: format_duration(track.get("duration_ms").and_then(Value::as_u64).unwrap_or(0)),
        explicit: track.get("explicit").and_then(Value::as_bool).unwrap_or(false),
        added: relative_date(added_at),
        added_by,
        local_file: false,
    })
}

async fn all_pages(api: &WebApi, base: &str, first: Value) -> Vec<Value> {
    pages_checked(api, base, first).await.0
}

async fn pages_checked(api: &WebApi, base: &str, first: Value) -> (Vec<Value>, bool) {
    let count = total(&first).max(first.get("total").and_then(Value::as_u64).unwrap_or(0));
    let mut out = items(&first);
    let fetched = out.len() as u64;
    let mut complete = true;
    if count > fetched {
        let pages = (fetched..count).step_by(PAGE as usize).map(|offset| {
            let sep = if base.contains('?') { '&' } else { '?' };
            let path = format!("{base}{sep}offset={offset}&limit={PAGE}");
            async move { api.get::<Value>(&path).await.map(|v| items(&v)) }
        });
        for page in join_all(pages).await {
            match page {
                Ok(rows) => out.extend(rows),
                Err(_) => complete = false,
            }
        }
    }
    (out, complete)
}

fn duration_sum(rows: &[Value], key: &str) -> u64 {
    rows.iter()
        .filter_map(|r| r.get(key).unwrap_or(r).get("duration_ms").and_then(Value::as_u64))
        .sum()
}

pub async fn playlist(api: &WebApi, me: &Me, id: &str) -> anyhow::Result<Collection> {
    let head: Value = api
        .get(&format!("/playlists/{id}?fields=id,uri,name,description,public,collaborative,images,owner(id,display_name),items(total)"))
        .await?;
    let base = format!("/playlists/{id}/items");
    let first: Value = api.get(&format!("{base}?limit={PAGE}")).await?;
    let (rows, complete) = pages_checked(api, &base, first).await;

    let owner_id = head.pointer("/owner/id").and_then(Value::as_str).unwrap_or_default();
    let adders: HashSet<String> = rows
        .iter()
        .filter_map(|r| r.pointer("/added_by/id").and_then(Value::as_str))
        .map(str::to_owned)
        .collect();
    let collaborative = head.get("collaborative").and_then(Value::as_bool).unwrap_or(false) || adders.len() > 1;
    let public = head.get("public").and_then(Value::as_bool).unwrap_or(true);

    let tracks: Vec<TrackRow> = rows
        .iter()
        .filter_map(|r| {
            let added_by = match r.pointer("/added_by/id").and_then(Value::as_str) {
                Some(uid) if uid == me.id => me.display_name.clone(),
                Some(uid) => uid.to_owned(),
                None => String::new(),
            };
            track_row(r.get("item")?, &s(r, "added_at"), added_by)
        })
        .collect();

    let count = total(&head).max(tracks.len() as u64);
    let mut meta = format!("{count} song{}", if count == 1 { "" } else { "s" });
    let duration = duration_sum(&rows, "item");
    if duration > 0 {
        meta.push_str(&format!(", {}", format_total(duration)));
    }

    Ok(Collection {
        id: id.to_owned(),
        uri: s(&head, "uri"),
        context: s(&head, "uri"),
        kind: "playlist".into(),
        label: if public { "Public Playlist".into() } else { "Private Playlist".into() },
        title: s(&head, "name"),
        description: crate::models::plain(&s(&head, "description")),
        owner: head.pointer("/owner/display_name").and_then(Value::as_str).unwrap_or_default().to_owned(),
        owner_is_me: owner_id == me.id,
        image: image(&head, 640),
        meta,
        show_album: true,
        show_added: !collaborative,
        show_added_by: collaborative,
        show_art: true,
        tracks,
        partial: !complete,
        ..Default::default()
    })
}

pub async fn liked(api: &WebApi, me: &Me) -> anyhow::Result<Collection> {
    let base = "/me/tracks";
    let first: Value = api.get(&format!("{base}?limit={PAGE}")).await?;
    let (rows, complete) = pages_checked(api, base, first).await;
    let tracks: Vec<TrackRow> = rows
        .iter()
        .filter_map(|r| track_row(r.get("track")?, &s(r, "added_at"), String::new()))
        .collect();
    let duration = duration_sum(&rows, "track");

    Ok(Collection {
        id: "liked".into(),
        uri: format!("spotify:user:{}:collection", me.id),
        context: "collection".into(),
        kind: "collection".into(),
        label: "Playlist".into(),
        title: "Liked Songs".into(),
        description: String::new(),
        owner: me.display_name.clone(),
        owner_is_me: true,
        image: None,
        meta: format!("{} songs, {}", tracks.len(), format_total(duration)),
        show_album: true,
        show_added: true,
        show_added_by: false,
        show_art: true,
        tracks,
        partial: !complete,
        ..Default::default()
    })
}

pub async fn liked_native(session: &librespot::core::session::Session, me: &Me) -> anyhow::Result<Collection> {
    use futures::stream::{self, StreamExt};

    static LOADING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _turn = LOADING.lock().await;
    let items: Vec<(String, i64)> = crate::native::collection_items(session, "collection")
        .await?
        .into_iter()
        .filter(|(u, _)| u.starts_with("spotify:track:"))
        .collect();
    if items.is_empty() {
        anyhow::bail!("liked songs are unavailable right now");
    }
    let uris: Vec<String> = items.iter().map(|(u, _)| u.clone()).collect();
    let mut cache = track_cache().lock().await.clone();
    let mut rows: Vec<Option<(TrackRow, u64)>> = uris.iter().map(|u| cache.get(u).cloned()).collect();
    let started = std::time::Instant::now();
    for (round, width) in [(0u64, 8usize), (2, 4), (4, 2), (6, 1)] {
        let missing: Vec<usize> = rows.iter().enumerate().filter(|(_, r)| r.is_none()).map(|(i, _)| i).collect();
        if missing.is_empty() || started.elapsed() > std::time::Duration::from_secs(18) {
            break;
        }
        if round > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(round)).await;
        }
        let targets: Vec<String> = missing.iter().map(|i| uris[*i].clone()).collect();
        let retried: Vec<Option<(TrackRow, u64)>> =
            stream::iter(targets).map(|uri| async move { liked_row(session, &uri).await }).buffered(width).collect().await;
        for (i, row) in missing.into_iter().zip(retried) {
            if let Some(found) = &row {
                cache.insert(uris[i].clone(), found.clone());
            }
            rows[i] = row;
        }
    }
    let partial = rows.iter().any(Option::is_none);
    if partial {
        let session = session.clone();
        let pending: Vec<String> = rows.iter().zip(&uris).filter(|(r, _)| r.is_none()).map(|(_, u)| u.clone()).collect();
        tokio::spawn(finish_liked(session, pending));
    }
    store_tracks(cache).await;
    for (row, (_, added)) in rows.iter_mut().zip(&items) {
        if let Some((track, _)) = row {
            track.added = relative_date(&iso_from_ms(added * 1000));
        }
    }
    let (tracks, durations): (Vec<TrackRow>, Vec<u64>) = rows.into_iter().flatten().unzip();
    tracing::info!("liked songs via library session: {} of {} tracks", tracks.len(), uris.len());
    let total: u64 = durations.iter().sum();

    Ok(Collection {
        id: "liked".into(),
        uri: format!("spotify:user:{}:collection", me.id),
        context: "collection".into(),
        kind: "collection".into(),
        label: "Playlist".into(),
        title: "Liked Songs".into(),
        owner: me.display_name.clone(),
        owner_is_me: true,
        meta: format!("{} songs, {}", items.len(), format_total(total)),
        show_album: true,
        show_added: true,
        show_art: true,
        tracks,
        partial,
        ..Default::default()
    })
}

type TrackCache = std::collections::HashMap<String, (TrackRow, u64)>;

fn track_cache() -> &'static tokio::sync::Mutex<TrackCache> {
    static CACHE: std::sync::OnceLock<tokio::sync::Mutex<TrackCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let path = crate::config::Config::data_dir().join("tracks.json");
        let loaded = std::fs::read(path).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default();
        tokio::sync::Mutex::new(loaded)
    })
}

async fn store_tracks(update: TrackCache) {
    let snapshot = {
        let mut cache = track_cache().lock().await;
        cache.extend(update);
        serde_json::to_vec(&*cache).ok()
    };
    let Some(raw) = snapshot else { return };
    let dir = crate::config::Config::data_dir();
    let _ = tokio::task::spawn_blocking(move || {
        let temp = dir.join("tracks.json.tmp");
        if std::fs::write(&temp, raw).is_ok() {
            let _ = std::fs::rename(&temp, dir.join("tracks.json"));
        }
    })
    .await;
}

pub async fn forget_tracks() {
    track_cache().lock().await.clear();
}

static FINISHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

async fn finish_liked(session: librespot::core::session::Session, pending: Vec<String>) {
    use std::sync::atomic::Ordering;
    if FINISHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut left = pending;
    for _ in 0..30 {
        if left.is_empty() {
            break;
        }
        let mut found = Vec::new();
        let mut still = Vec::new();
        for uri in left {
            match liked_row(&session, &uri).await {
                Some(row) => found.push((uri, row)),
                None => still.push(uri),
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        if !found.is_empty() {
            store_tracks(found.into_iter().collect()).await;
        }
        left = still;
        if !left.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
    tracing::info!("liked songs cache finished, {} still missing", left.len());
    FINISHING.store(false, Ordering::SeqCst);
}

async fn liked_row(session: &librespot::core::session::Session, uri: &str) -> Option<(TrackRow, u64)> {
    use librespot::core::SpotifyUri;
    use librespot::metadata::{Metadata, Track};
    let parsed = SpotifyUri::from_uri(uri).ok()?;
    let track = Track::get(session, &parsed).await.ok()?;
    let id = parsed.to_id().ok()?;
    let ms = track.duration.max(0) as u64;
    Some((
        TrackRow {
            uri: format!("spotify:track:{id}"),
            id,
            album_id: track.album.id.to_id().unwrap_or_default(),
            name: track.name.clone(),
            artists: track.artists.iter().map(|a| ArtistRef { id: a.id.to_id().unwrap_or_default(), name: a.name.clone() }).collect(),
            album: track.album.name.clone(),
            image: cover_url(&track.album.covers, 64),
            duration: format_duration(ms),
            explicit: track.is_explicit,
            added: String::new(),
            added_by: String::new(),
            local_file: false,
        },
        ms,
    ))
}

fn long_date(raw: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
    ];
    let parts: Vec<&str> = raw.split('-').collect();
    match (parts.first(), parts.get(1).and_then(|m| m.parse::<usize>().ok()), parts.get(2).and_then(|d| d.parse::<u32>().ok())) {
        (Some(year), Some(m @ 1..=12), Some(day)) => format!("{} {day}, {year}", MONTHS[m - 1]),
        _ => raw.to_owned(),
    }
}

pub async fn album(api: &WebApi, id: &str) -> anyhow::Result<Collection> {
    let head: Value = api.get(&format!("/albums/{id}")).await?;
    let owner_id = head.pointer("/artists/0/id").and_then(Value::as_str).unwrap_or_default().to_owned();
    let owner_image = if owner_id.is_empty() {
        None
    } else {
        api.get::<Value>(&format!("/artists/{owner_id}")).await.ok().and_then(|a| image(&a, 64))
    };
    let mut footer = vec![long_date(&s(&head, "release_date"))];
    footer.extend(
        head.get("copyrights")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|c| {
                let text = s(c, "text");
                match s(c, "type").as_str() {
                    "P" if !text.starts_with('℗') => format!("℗ {text}"),
                    "C" if !text.starts_with('©') => format!("© {text}"),
                    _ => text,
                }
            })
            .filter(|t| !t.trim().is_empty()),
    );
    let base = format!("/albums/{id}/tracks");
    let first = head.get("tracks").cloned().unwrap_or_default();
    let (rows, complete) = pages_checked(api, &base, first).await;
    let tracks: Vec<TrackRow> = rows.iter().filter_map(|t| track_row(t, "", String::new())).collect();

    let year = s(&head, "release_date").chars().take(4).collect::<String>();
    let duration = duration_sum(&rows, "_");
    let artist_names = artists(&head).into_iter().map(|a| a.name).collect::<Vec<_>>().join(", ");
    let kind = crate::models::release_label(&s(&head, "album_type"), tracks.len() as u64);

    Ok(Collection {
        id: id.to_owned(),
        uri: s(&head, "uri"),
        context: s(&head, "uri"),
        kind: "album".into(),
        label: kind,
        title: s(&head, "name"),
        description: String::new(),
        owner: artist_names,
        owner_is_me: false,
        image: image(&head, 640),
        meta: format!("{year} • {} song{}, {}", tracks.len(), if tracks.len() == 1 { "" } else { "s" }, format_total(duration)),
        show_album: false,
        show_added: false,
        show_added_by: false,
        show_art: false,
        tracks,
        owner_id,
        owner_image,
        footer,
        more: Vec::new(),
        partial: !complete,
    })
}

impl PlaylistIndex {
    pub async fn set_options(&self, options: Vec<PlaylistOption>) {
        if !options.is_empty() {
            *self.playlists.write().await = options;
        }
    }

    pub async fn options(&self) -> Vec<PlaylistOption> {
        self.playlists.read().await.clone()
    }

    pub async fn containing(&self, track_id: &str) -> Vec<String> {
        self.entries
            .read()
            .await
            .iter()
            .filter(|(_, e)| e.tracks.contains(track_id))
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub async fn refresh(&self, api: &WebApi, me: &Me) -> anyhow::Result<()> {
        if self.entries.read().await.is_empty() {
            let saved: HashMap<String, Indexed> = std::fs::read(index_path()).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default();
            if !saved.is_empty() {
                *self.entries.write().await = saved;
            }
        }
        let first: Value = api.get(&format!("/me/playlists?limit={PAGE}")).await?;
        let all = all_pages(api, "/me/playlists", first).await;
        let editable: Vec<&Value> = all
            .iter()
            .filter(|p| {
                p.pointer("/owner/id").and_then(Value::as_str) == Some(me.id.as_str())
                    || p.get("collaborative").and_then(Value::as_bool).unwrap_or(false)
            })
            .collect();

        *self.playlists.write().await = editable
            .iter()
            .map(|p| PlaylistOption {
                id: s(p, "id"),
                uri: s(p, "uri"),
                name: s(p, "name"),
                image: image(p, 64),
                total: total(p),
            })
            .collect();

        let stale: Vec<(String, String)> = {
            let entries = self.entries.read().await;
            editable
                .iter()
                .map(|p| (s(p, "id"), s(p, "snapshot_id")))
                .filter(|(id, snap)| entries.get(id).is_none_or(|e| &e.snapshot != snap))
                .collect()
        };

        let fetched = join_all(stale.into_iter().map(|(id, snapshot)| async move {
            let base = format!("/playlists/{id}/items");
            let first = api.get::<Value>(&format!("{base}?limit={PAGE}")).await.ok()?;
            let (rows, complete) = pages_checked(api, &base, first).await;
            if !complete {
                return None;
            }
            let tracks = rows
                .iter()
                .filter_map(|r| r.pointer("/item/id").and_then(Value::as_str).map(str::to_owned))
                .collect();
            Some((id, Indexed { snapshot, tracks }))
        }))
        .await;

        let mut entries = self.entries.write().await;
        let keep: HashSet<String> = editable.iter().map(|p| s(p, "id")).collect();
        entries.retain(|id, _| keep.contains(id));
        for (id, indexed) in fetched.into_iter().flatten() {
            entries.insert(id, indexed);
        }
        tracing::info!("indexed {} editable playlists", entries.len());
        if let Ok(raw) = serde_json::to_vec(&*entries) {
            let _ = std::fs::write(index_path(), raw);
        }
        Ok(())
    }

    async fn mark(&self, playlist: &str, track: &str, present: bool) {
        if let Some(e) = self.entries.write().await.get_mut(playlist) {
            e.snapshot.clear();
            if present {
                e.tracks.insert(track.to_owned());
            } else {
                e.tracks.remove(track);
            }
        }
        if let Some(p) = self.playlists.write().await.iter_mut().find(|p| p.id == playlist) {
            p.total = if present { p.total + 1 } else { p.total.saturating_sub(1) };
        }
    }

    pub async fn insert_playlist(&self, option: PlaylistOption) {
        self.entries.write().await.insert(
            option.id.clone(),
            Indexed { snapshot: String::new(), tracks: HashSet::new() },
        );
        self.playlists.write().await.insert(0, option);
    }

    pub async fn forget(&self) {
        self.entries.write().await.clear();
        self.playlists.write().await.clear();
        let _ = std::fs::remove_file(index_path());
    }

    pub async fn remove_playlist(&self, id: &str) {
        self.entries.write().await.remove(id);
        self.playlists.write().await.retain(|p| p.id != id);
    }
}

pub async fn is_liked(api: &WebApi, track_id: &str) -> anyhow::Result<bool> {
    let found: Vec<bool> = api
        .get(&format!("/me/library/contains?uris=spotify:track:{track_id}"))
        .await?;
    Ok(found.first().copied().unwrap_or(false))
}

pub async fn set_liked(api: &WebApi, track_id: &str, liked: bool) -> anyhow::Result<()> {
    let path = format!("/me/library?uris=spotify:track:{track_id}");
    let result = if liked { api.put(&path, None).await } else { api.delete(&path, None).await };
    if result.is_ok() {
        return result;
    }
    let legacy = format!("/me/tracks?ids={track_id}");
    if liked { api.put(&legacy, None).await } else { api.delete(&legacy, None).await }
}

pub async fn set_in_playlist(
    api: &WebApi,
    session: Option<&librespot::core::session::Session>,
    index: &PlaylistIndex,
    playlist: &str,
    track_id: &str,
    present: bool,
) -> anyhow::Result<()> {
    let uri = format!("spotify:track:{track_id}");
    let path = format!("/playlists/{playlist}/items");
    let web = if present {
        api.post(&path, Some(json!({ "uris": [uri] }))).await.map(|_| ())
    } else {
        match api.delete(&path, Some(json!({ "items": [{ "uri": uri }] }))).await {
            Ok(()) => Ok(()),
            Err(_) => api.delete(&path, Some(json!({ "tracks": [{ "uri": uri }] }))).await,
        }
    };
    if let Err(web_err) = web {
        let Some(session) = session else { return Err(web_err) };
        crate::native::playlist_change(session, playlist, &uri, present)
            .await
            .inspect_err(|e| tracing::warn!("playlist {playlist} via web: {web_err:#}; native: {e:#}"))?;
    }
    index.mark(playlist, track_id, present).await;
    Ok(())
}

pub async fn create_playlist(api: &WebApi, me: &Me, name: &str) -> anyhow::Result<PlaylistOption> {
    let body = json!({ "name": name, "public": false });
    let created = match api.post("/me/playlists", Some(body.clone())).await {
        Ok(v) => v,
        Err(first) => api
            .post(&format!("/users/{}/playlists", me.id), Some(body))
            .await
            .inspect_err(|e| tracing::warn!("create playlist: {first:#}; fallback: {e:#}"))?,
    };
    Ok(PlaylistOption {
        id: s(&created, "id"),
        uri: s(&created, "uri"),
        name: s(&created, "name"),
        image: None,
        total: 0,
    })
}

pub async fn set_library_item(api: &WebApi, uri: &str, saved: bool) -> anyhow::Result<()> {
    let path = format!("/me/library?uris={uri}");
    if saved { api.put(&path, None).await } else { api.delete(&path, None).await }
}

pub async fn in_library(api: &WebApi, uri: &str) -> anyhow::Result<bool> {
    let found: Vec<bool> = api.get(&format!("/me/library/contains?uris={uri}")).await?;
    Ok(found.first().copied().unwrap_or(false))
}

pub fn cover_url(images: &librespot::metadata::image::Images, min: i32) -> Option<String> {
    let mut sorted: Vec<_> = images.iter().collect();
    sorted.sort_by_key(|i| i.width);
    sorted
        .iter()
        .find(|i| i.width >= min)
        .or_else(|| sorted.last())
        .map(|i| format!("https://i.scdn.co/image/{}", i.id))
}

fn iso_from_ms(ms: i64) -> String {
    librespot::core::date::Date::from_timestamp_ms(ms)
        .map(|d| {
            let dt = d.as_utc();
            format!("{:04}-{:02}-{:02}", dt.year(), u8::from(dt.month()), dt.day())
        })
        .unwrap_or_default()
}

pub async fn playlist_cover_native(session: &librespot::core::session::Session, id: &str) -> Option<String> {
    use librespot::core::{SpotifyUri, spotify_id::SpotifyId};
    use librespot::metadata::{Metadata, Playlist, Track};

    let uri = SpotifyUri::Playlist { user: None, id: SpotifyId::from_base62(id).ok()? };
    let list = Playlist::get(session, &uri).await.ok()?;
    let mut covers: Vec<String> = Vec::new();
    for item in list.contents.items.iter().filter(|i| matches!(i.id, SpotifyUri::Track { .. })).take(12) {
        let Ok(track) = Track::get(session, &item.id).await else { continue };
        let images = if track.album.covers.is_empty() { &track.album.cover_group } else { &track.album.covers };
        if let Some(url) = cover_url(images, 300)
            && !covers.contains(&url) {
                covers.push(url);
            }
        if covers.len() == 4 {
            break;
        }
    }
    if covers.len() == 4 {
        let ids: String = covers.iter().filter_map(|u| u.rsplit('/').next()).collect();
        return Some(format!("https://mosaic.scdn.co/640/{ids}"));
    }
    covers.into_iter().next()
}

fn mosaic(tracks: &[TrackRow]) -> Option<String> {
    let mut covers: Vec<String> = Vec::new();
    for image in tracks.iter().filter_map(|t| t.image.as_deref()) {
        let image = image.replace("ab67616d00004851", "ab67616d00001e02");
        if !covers.contains(&image) {
            covers.push(image);
        }
        if covers.len() == 4 {
            let ids: String = covers.iter().filter_map(|u| u.rsplit('/').next()).collect();
            return Some(format!("https://mosaic.scdn.co/640/{ids}"));
        }
    }
    covers.into_iter().next()
}

pub async fn playlist_native(
    session: &librespot::core::session::Session,
    me: &Me,
    id: &str,
) -> anyhow::Result<Collection> {
    use futures::StreamExt;
    use librespot::core::{SpotifyUri, spotify_id::SpotifyId};
    use librespot::metadata::{Metadata, Playlist, Track};

    let playlist_id = SpotifyId::from_base62(id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let uri = SpotifyUri::Playlist { user: None, id: playlist_id };
    let list = Playlist::get(session, &uri).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let attrs = &list.attributes;

    let entries: Vec<(SpotifyUri, i64, String)> = list
        .contents
        .items
        .iter()
        .filter(|i| matches!(i.id, SpotifyUri::Track { .. }))
        .map(|i| (i.id.clone(), i.attributes.timestamp.as_timestamp_ms(), i.attributes.added_by.clone()))
        .collect();

    let tracks: Vec<TrackRow> = futures::stream::iter(entries)
        .map(|(track_uri, added_ms, added_by)| async move {
            let track = Track::get(session, &track_uri).await.ok()?;
            let id = track_uri.to_id().ok()?;
            let covers = if track.album.covers.is_empty() { &track.album.cover_group } else { &track.album.covers };
            Some(TrackRow {
                uri: format!("spotify:track:{id}"),
                album_id: track.album.id.to_id().unwrap_or_default(),
                id,
                name: track.name.clone(),
                artists: track
                    .artists
                    .iter()
                    .map(|a| ArtistRef { id: a.id.to_id().unwrap_or_default(), name: a.name.clone() })
                    .collect(),
                album: track.album.name.clone(),
                image: cover_url(covers, 64),
                duration: format_duration(track.duration.max(0) as u64),
                explicit: track.is_explicit,
                added: if added_ms > 0 { relative_date(&iso_from_ms(added_ms)) } else { String::new() },
                added_by: if added_by == me.id { me.display_name.clone() } else { added_by },
                local_file: false,
            })
        })
        .buffered(8)
        .filter_map(|row| async move { row })
        .collect()
        .await;

    let image = ["large", "default", "small"]
        .iter()
        .find_map(|want| attrs.picture_sizes.iter().find(|p| p.target_name == *want).map(|p| p.url.clone()))
        .or_else(|| attrs.picture_sizes.first().map(|p| p.url.clone()))
        .or_else(|| {
            (!attrs.picture.is_empty()).then(|| {
                let hex: String = attrs.picture.iter().map(|b| format!("{b:02x}")).collect();
                format!("https://i.scdn.co/image/{hex}")
            })
        })
        .or_else(|| {
            ["image_url", "image"]
                .iter()
                .find_map(|key| attrs.format_attributes.get(*key))
                .and_then(|raw| crate::extras::spotify_image(raw))
        })
        .filter(|u| !u.is_empty())
        .or_else(|| mosaic(&tracks));
    let owner_id = match &list.id {
        SpotifyUri::Playlist { user: Some(user), .. } => user.clone(),
        _ => String::new(),
    };
    let owner_is_me = !owner_id.is_empty() && owner_id == me.id;

    let made_for = !attrs.format.is_empty();
    let any_added = tracks.iter().any(|t| !t.added.is_empty());
    let count = tracks.len();

    Ok(Collection {
        id: id.to_owned(),
        uri: format!("spotify:playlist:{id}"),
        context: format!("spotify:playlist:{id}"),
        kind: "playlist".into(),
        label: "Playlist".into(),
        title: attrs.name.clone(),
        description: crate::models::plain(&attrs.description),
        owner: if made_for || id.starts_with("37i9") || owner_id == "spotify" {
            "Spotify".into()
        } else if owner_is_me {
            me.display_name.clone()
        } else {
            owner_id.clone()
        },
        owner_is_me,
        owner_image: if owner_is_me { me.image.clone() } else { None },
        owner_id,
        image,
        meta: if made_for {
            format!("Made for {} • {count} songs", me.display_name)
        } else {
            format!("{count} song{}", if count == 1 { "" } else { "s" })
        },
        show_album: true,
        show_added: any_added,
        show_added_by: false,
        show_art: true,
        tracks,
        ..Default::default()
    })
}
