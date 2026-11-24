use crate::collection::{TrackRow, artists, track_row};
use crate::models::{ArtistRef, Card, image, s};
use crate::web::WebApi;
use futures::future::join_all;
use librespot::{
    core::{SpotifyUri, session::Session, spotify_id::SpotifyId},
    metadata::{Album, Artist, Metadata},
};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct ArtistPage {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub followers: Option<u64>,
    pub following: bool,
    pub listeners: String,
    pub verified: bool,
    pub top: Vec<TrackRow>,
    pub albums: Vec<Card>,
    pub singles: Vec<Card>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackInfo {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub album_id: String,
    pub album: String,
    pub image: Option<String>,
    pub release: String,
    pub label: String,
    pub copyright: String,
}

async fn top_track_ids(session: &Session, id: &str) -> Vec<String> {
    let Ok(artist_id) = SpotifyId::from_base62(id) else { return Vec::new() };
    let Ok(artist) = Artist::get(session, &SpotifyUri::Artist { id: artist_id }).await else {
        return Vec::new();
    };
    let country = session.country();
    artist
        .top_tracks
        .iter()
        .find(|t| t.country == country)
        .or_else(|| artist.top_tracks.first())
        .map(|t| t.tracks.iter().filter_map(|u| u.to_id().ok()).take(10).collect())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
pub struct Release {
    pub label: String,
    pub card: Card,
    pub date: String,
    pub single: bool,
    pub artist: String,
    pub large: Option<String>,
}

async fn release(session: &Session, uri: &SpotifyUri, single: bool) -> Option<Release> {
    let album = Album::get(session, uri).await.ok()?;
    let id = uri.to_id().ok()?;
    let when = album.date.as_utc();
    let date = format!("{:04}-{:02}-{:02}", when.year(), u8::from(when.month()), when.day());
    let kind = crate::models::release_label(if single { "single" } else { "album" }, album.tracks().count() as u64);
    let artist = album.artists.first().map(|a| a.name.clone()).unwrap_or_default();
    Some(Release {
        card: Card {
            id: id.clone(),
            uri: format!("spotify:album:{id}"),
            kind: "album".into(),
            title: album.name.clone(),
            subtitle: format!("{} • {kind}", when.year()),
            image: crate::collection::cover_url(&album.covers, 300),
            pinned: false,
        },
        date,
        single,
        label: kind,
        artist,
        large: crate::collection::cover_url(&album.covers, 640),
    })
}

pub async fn releases(session: &Session, artist_id: &str, per_group: usize) -> Vec<Release> {
    let Ok(id) = SpotifyId::from_base62(artist_id) else { return Vec::new() };
    let Ok(artist) = Artist::get(session, &SpotifyUri::Artist { id }).await else {
        return Vec::new();
    };
    let wanted = artist
        .albums
        .current_releases()
        .take(per_group)
        .map(|u| (u.clone(), false))
        .chain(artist.singles.current_releases().take(per_group).map(|u| (u.clone(), true)))
        .collect::<Vec<_>>();
    let mut out: Vec<Release> = join_all(wanted.iter().map(|(uri, single)| release(session, uri, *single)))
        .await
        .into_iter()
        .flatten()
        .collect();
    out.sort_by(|a, b| b.date.cmp(&a.date));
    out
}

pub async fn page(api: &WebApi, session: Option<Session>, id: &str) -> anyhow::Result<ArtistPage> {
    let uri = format!("spotify:artist:{id}");
    let head_path = format!("/artists/{id}");
    let (head, native, top_ids) = tokio::join!(
        api.get::<Value>(&head_path),
        async {
            match &session {
                Some(session) => releases(session, id, 12).await,
                None => Vec::new(),
            }
        },
        async {
            match &session {
                Some(session) => top_track_ids(session, id).await,
                None => Vec::new(),
            }
        },
    );
    let head = head?;

    let top = join_all(top_ids.iter().map(|tid| async move {
        let track = api.get::<Value>(&format!("/tracks/{tid}")).await.ok()?;
        track_row(&track, "", String::new())
    }))
    .await
    .into_iter()
    .flatten()
    .collect();

    let (albums, singles) = if native.is_empty() {
        let releases: Vec<Value> = api
            .get::<Value>(&format!("/artists/{id}/albums?include_groups=album,single&limit=10"))
            .await
            .inspect_err(|e| tracing::warn!("artist releases: {e:#}"))
            .ok()
            .and_then(|v| v.get("items").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        let (albums, singles): (Vec<Value>, Vec<Value>) = releases.into_iter().partition(|r| s(r, "album_type") == "album");
        (albums.iter().map(Card::album).collect(), singles.iter().map(Card::album).collect())
    } else {
        let (singles, albums): (Vec<Release>, Vec<Release>) = native.into_iter().partition(|r| r.single);
        (albums.into_iter().map(|r| r.card).collect::<Vec<_>>(), singles.into_iter().map(|r| r.card).collect::<Vec<_>>())
    };

    Ok(ArtistPage {
        id: id.to_owned(),
        uri,
        name: s(&head, "name"),
        image: image(&head, 640),
        followers: head.pointer("/followers/total").and_then(Value::as_u64),
        following: false,
        listeners: String::new(),
        verified: false,
        top,
        albums,
        singles,
    })
}

pub async fn track_info(api: &WebApi, id: &str) -> anyhow::Result<TrackInfo> {
    let track: Value = api.get(&format!("/tracks/{id}")).await?;
    let album_id = track.pointer("/album/id").and_then(Value::as_str).unwrap_or_default().to_owned();
    let album: Value = if album_id.is_empty() {
        Value::Null
    } else {
        api.get(&format!("/albums/{album_id}")).await.unwrap_or(Value::Null)
    };
    let copyright = album
        .get("copyrights")
        .and_then(Value::as_array)
        .and_then(|c| c.first())
        .map(|c| s(c, "text"))
        .unwrap_or_default();

    Ok(TrackInfo {
        id: s(&track, "id"),
        uri: s(&track, "uri"),
        name: s(&track, "name"),
        artists: artists(&track),
        album: track.pointer("/album/name").and_then(Value::as_str).unwrap_or_default().to_owned(),
        image: track.get("album").and_then(|a| image(a, 300)),
        release: track.pointer("/album/release_date").and_then(Value::as_str).unwrap_or_default().to_owned(),
        label: s(&album, "label"),
        copyright,
        album_id,
    })
}
