use librespot::core::session::Session;
use reqwest::{Method, header::HeaderMap};
use serde_json::Value;
use std::collections::HashMap;

fn collection_body(user: &str, set: &str) -> Vec<u8> {
    let mut body = Vec::new();
    for (field, value) in [(1u8, user), (2u8, set)] {
        body.push((field << 3) | 2);
        body.push(value.len() as u8);
        body.extend_from_slice(value.as_bytes());
    }
    body.extend_from_slice(&[4 << 3, 100]);
    body
}

fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn next_token(bytes: &[u8]) -> Option<String> {
    let mut at = 0;
    while at < bytes.len() {
        let key = varint(bytes, &mut at)?;
        match key & 7 {
            0 => {
                varint(bytes, &mut at)?;
            }
            2 => {
                let len = varint(bytes, &mut at)? as usize;
                let value = bytes.get(at..at + len)?;
                at += len;
                if key >> 3 == 2 && !value.is_empty() {
                    return String::from_utf8(value.to_vec()).ok();
                }
            }
            1 => at += 8,
            5 => at += 4,
            _ => return None,
        }
    }
    None
}

fn paged_body(user: &str, set: &str, token: Option<&str>) -> Vec<u8> {
    let mut body = Vec::new();
    let mut fields = vec![(1u8, user), (2u8, set)];
    if let Some(token) = token {
        fields.push((3u8, token));
    }
    for (field, value) in fields {
        body.push((field << 3) | 2);
        let mut len = value.len();
        loop {
            let byte = (len & 0x7f) as u8;
            len >>= 7;
            if len == 0 {
                body.push(byte);
                break;
            }
            body.push(byte | 0x80);
        }
        body.extend_from_slice(value.as_bytes());
    }
    body.extend_from_slice(&[4 << 3, 0xac, 0x02]);
    body
}

fn page_items(bytes: &[u8]) -> Vec<(String, i64, bool)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let Some(key) = varint(bytes, &mut at) else { break };
        match key & 7 {
            0 => {
                if varint(bytes, &mut at).is_none() {
                    break;
                }
            }
            2 => {
                let Some(len) = varint(bytes, &mut at).map(|l| l as usize) else { break };
                let Some(value) = bytes.get(at..at + len) else { break };
                at += len;
                if key >> 3 == 1 {
                    let (mut uri, mut added, mut removed) = (String::new(), 0i64, false);
                    let mut inner = 0;
                    while inner < value.len() {
                        let Some(k) = varint(value, &mut inner) else { break };
                        match (k >> 3, k & 7) {
                            (1, 2) => {
                                let Some(l) = varint(value, &mut inner).map(|l| l as usize) else { break };
                                uri = String::from_utf8_lossy(value.get(inner..inner + l).unwrap_or_default()).into_owned();
                                inner += l;
                            }
                            (2, 0) => added = varint(value, &mut inner).unwrap_or(0) as i64,
                            (3, 0) => removed = varint(value, &mut inner).unwrap_or(0) != 0,
                            (_, 0) => {
                                varint(value, &mut inner);
                            }
                            (_, 2) => {
                                let l = varint(value, &mut inner).unwrap_or(0) as usize;
                                inner += l;
                            }
                            (_, 1) => inner += 8,
                            (_, 5) => inner += 4,
                            _ => break,
                        }
                    }
                    if uri.starts_with("spotify:") {
                        out.push((uri, added, removed));
                    }
                }
            }
            1 => at += 8,
            5 => at += 4,
            _ => break,
        }
    }
    out
}

pub async fn collection_items(session: &Session, set: &str) -> anyhow::Result<Vec<(String, i64)>> {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/vnd.collection-v2.spotify.proto".parse()?);
    let user = session.username();
    let mut out: Vec<(String, i64)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut token: Option<String> = None;
    for _ in 0..40 {
        let body = paged_body(&user, set, token.as_deref());
        let mut attempt = 0u32;
        let bytes = loop {
            match session.spclient().request(&Method::POST, "/collection/v2/paging", Some(headers.clone()), Some(&body)).await {
                Ok(bytes) => break bytes,
                Err(e) if attempt < 5 => {
                    attempt += 1;
                    tracing::debug!("collection {set} page retry {attempt}: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(500 * 2u64.pow(attempt))).await;
                }
                Err(e) => anyhow::bail!("collection {set} is unavailable right now: {e}"),
            }
        };
        for (uri, added, removed) in page_items(&bytes) {
            if !removed && seen.insert(uri.clone()) {
                out.push((uri, added));
            }
        }
        match next_token(&bytes) {
            Some(next) if Some(&next) != token.as_ref() => token = Some(next),
            _ => break,
        }
    }
    out.sort_by_key(|item| std::cmp::Reverse(item.1));
    Ok(out)
}

pub async fn collection_set(session: &Session, set: &str) -> Vec<String> {
    match collection_items(session, set).await {
        Ok(items) => items.into_iter().map(|(uri, _)| uri).collect(),
        Err(e) => {
            tracing::warn!("{e:#}");
            Vec::new()
        }
    }
}

pub async fn collection_checked(session: &Session, set: &str) -> anyhow::Result<Vec<String>> {
    Ok(collection_items(session, set).await?.into_iter().map(|(uri, _)| uri).collect())
}

fn scan_uris(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    let mut rest = text.as_ref();
    while let Some(pos) = rest.find("spotify:") {
        let tail = &rest[pos..];
        let end = tail
            .char_indices()
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == ':'))
            .map(|(i, _)| i)
            .unwrap_or(tail.len());
        let uri = &tail[..end];
        if uri.matches(':').count() == 2 && uri.len() > 12 && !out.contains(&uri.to_owned()) {
            out.push(uri.to_owned());
        }
        rest = &tail[end.max(1)..];
    }
    out
}

pub async fn recently_played(session: &Session) -> HashMap<String, i64> {
    let path = format!(
        "/recently-played/v3/user/{}/recently-played?format=json&offset=0&limit=50&filter=default,collection-new-episodes",
        session.username()
    );
    let mut map = HashMap::new();
    let Ok(bytes) = session.spclient().request_as_json(&Method::GET, &path, None, None).await else {
        return map;
    };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        return map;
    };
    for ctx in v.get("playContexts").and_then(Value::as_array).into_iter().flatten() {
        if let (Some(uri), Some(time)) = (
            ctx.get("uri").and_then(Value::as_str),
            ctx.get("lastPlayedTime").and_then(Value::as_i64),
        ) {
            map.insert(uri.to_owned(), time);
        }
    }
    map
}

pub async fn pins(session: &Session) -> Vec<String> {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/vnd.collection-v2.spotify.proto".parse().unwrap());
    let body = collection_body(&session.username(), "ylpin");
    match session
        .spclient()
        .request(&Method::POST, "/collection/v2/paging", Some(headers), Some(&body))
        .await
    {
        Ok(bytes) => scan_uris(&bytes)
            .into_iter()
            .filter(|u| !u.starts_with("spotify:collection"))
            .collect(),
        Err(e) => {
            tracing::warn!("pins: {e}");
            Vec::new()
        }
    }
}

pub struct RootItem {
    pub uri: String,
    pub made_for: bool,
    pub name: String,
    pub image: Option<String>,
    pub owner: String,
    pub added: i64,
}

pub async fn rootlist(session: &Session) -> Vec<RootItem> {
    use librespot::protocol::playlist4_external::{ListAttributes, SelectedListContent};
    use protobuf::Message;

    let bytes = match session.spclient().get_rootlist(0, Some(500)).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("rootlist: {e}");
            return Vec::new();
        }
    };
    let Ok(list) = SelectedListContent::parse_from_bytes(&bytes) else {
        tracing::warn!("rootlist: parse failed");
        return Vec::new();
    };
    let contents = list.contents.get_or_default();
    let aligned = contents.items.len() == contents.meta_items.len();

    let image = |attrs: &ListAttributes| -> Option<String> {
        let sized = ["large", "default", "small"].iter().find_map(|want| {
            attrs.picture_size.iter().find(|p| p.target_name() == *want).map(|p| p.url().to_owned())
        });
        sized
            .or_else(|| attrs.picture_size.first().map(|p| p.url().to_owned()))
            .or_else(|| {
                attrs
                    .format_attributes
                    .iter()
                    .find(|f| f.key() == "image_url" || f.key() == "image")
                    .and_then(|f| crate::extras::spotify_image(f.value()))
            })
            .or_else(|| {
                (!attrs.picture().is_empty()).then(|| {
                    let hex: String = attrs.picture().iter().map(|b| format!("{b:02x}")).collect();
                    format!("https://i.scdn.co/image/{hex}")
                })
            })
            .filter(|u| !u.is_empty())
    };

    contents
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.uri().starts_with("spotify:playlist:"))
        .map(|(i, item)| {
            let meta = aligned.then(|| &contents.meta_items[i]);
            let attrs = meta.map(|m| m.attributes.get_or_default());
            RootItem {
                uri: item.uri().to_owned(),
                made_for: attrs.is_some_and(|a| !a.format().is_empty()),
                name: attrs.map(|a| a.name().to_owned()).unwrap_or_default(),
                image: attrs.and_then(image),
                owner: meta.map(|m| m.owner_username().to_owned()).unwrap_or_default(),
                added: item.attributes.get_or_default().timestamp(),
            }
        })
        .collect()
}

pub struct NativeLibrary {
    pub albums: Vec<crate::models::Card>,
    pub artists: Vec<crate::models::Card>,
    pub shows: Vec<crate::models::Card>,
    pub liked: u64,
}

pub async fn library(session: &Session) -> NativeLibrary {
    use crate::models::Card;
    use futures::future::join_all;
    use librespot::core::SpotifyUri;
    use librespot::metadata::{Album, Artist, Metadata, Show};

    let (collection, artists, shows) =
        tokio::join!(collection_set(session, "collection"), collection_set(session, "artist"), collection_set(session, "show"));
    let liked = collection.iter().filter(|u| u.starts_with("spotify:track:")).count() as u64;
    let parse = |uri: &str| SpotifyUri::from_uri(uri).ok();

    let albums = join_all(collection.iter().filter(|u| u.starts_with("spotify:album:")).take(60).filter_map(|u| parse(u)).map(|uri| async move {
        let album = Album::get(session, &uri).await.ok()?;
        let id = uri.to_id().ok()?;
        Some(Card {
            uri: format!("spotify:album:{id}"),
            id,
            kind: "album".into(),
            title: album.name.clone(),
            subtitle: format!("Album • {}", album.artists.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ")),
            image: crate::collection::cover_url(&album.covers, 300),
            pinned: false,
        })
    }))
    .await
    .into_iter()
    .flatten()
    .collect();

    let artists = join_all(artists.iter().filter(|u| u.starts_with("spotify:artist:")).take(60).filter_map(|u| parse(u)).map(|uri| async move {
        let artist = Artist::get(session, &uri).await.ok()?;
        let id = uri.to_id().ok()?;
        let image = crate::collection::cover_url(&artist.portraits, 300).or_else(|| crate::collection::cover_url(&artist.portrait_group, 300));
        Some(Card {
            uri: format!("spotify:artist:{id}"),
            id,
            kind: "artist".into(),
            title: artist.name.clone(),
            subtitle: "Artist".into(),
            image,
            pinned: false,
        })
    }))
    .await
    .into_iter()
    .flatten()
    .collect();

    let shows = join_all(shows.iter().filter(|u| u.starts_with("spotify:show:")).take(40).filter_map(|u| parse(u)).map(|uri| async move {
        let show = Show::get(session, &uri).await.ok()?;
        let id = uri.to_id().ok()?;
        Some(Card {
            uri: format!("spotify:show:{id}"),
            id,
            kind: "show".into(),
            title: show.name.clone(),
            subtitle: if show.publisher.is_empty() { "Podcast".into() } else { format!("Podcast • {}", show.publisher) },
            image: crate::collection::cover_url(&show.covers, 300),
            pinned: false,
        })
    }))
    .await
    .into_iter()
    .flatten()
    .collect();

    NativeLibrary { albums, artists, shows, liked }
}

fn field_bytes(body: &mut Vec<u8>, field: u8, value: &[u8]) {
    body.push((field << 3) | 2);
    let mut len = value.len();
    loop {
        let byte = (len & 0x7f) as u8;
        len >>= 7;
        if len == 0 {
            body.push(byte);
            break;
        }
        body.push(byte | 0x80);
    }
    body.extend_from_slice(value);
}

fn field_varint(body: &mut Vec<u8>, field: u8, mut value: u64) {
    body.push(field << 3);
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            body.push(byte);
            break;
        }
        body.push(byte | 0x80);
    }
}

pub async fn collection_write(session: &Session, uri: &str, saved: bool) -> anyhow::Result<()> {
    let set = match uri.split(':').nth(1).unwrap_or_default() {
        "track" | "album" => "collection",
        "artist" => "artist",
        "show" => "show",
        "episode" => "listenlater",
        other => anyhow::bail!("can't save {other} items"),
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut item = Vec::new();
    field_bytes(&mut item, 1, uri.as_bytes());
    field_varint(&mut item, 2, now);
    if !saved {
        field_varint(&mut item, 3, 1);
    }
    let mut body = Vec::new();
    field_bytes(&mut body, 1, session.username().as_bytes());
    field_bytes(&mut body, 2, set.as_bytes());
    field_bytes(&mut body, 3, &item);
    field_bytes(&mut body, 4, uuid::Uuid::new_v4().simple().to_string().as_bytes());
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/vnd.collection-v2.spotify.proto".parse()?);
    session.spclient().request(&Method::POST, "/collection/v2/write", Some(headers), Some(&body)).await?;
    Ok(())
}

pub async fn show_publisher(session: &Session, id: &str) -> Option<String> {
    use librespot::core::SpotifyUri;
    use librespot::metadata::{Metadata, Show};
    let uri = SpotifyUri::from_uri(&format!("spotify:show:{id}")).ok()?;
    let show = Show::get(session, &uri).await.ok()?;
    (!show.publisher.is_empty()).then(|| show.publisher.clone())
}

fn rem_op(index: usize, uri: &str) -> Vec<u8> {
    let mut item = Vec::new();
    field_bytes(&mut item, 1, uri.as_bytes());
    let mut body = Vec::new();
    field_varint(&mut body, 1, index as u64);
    field_varint(&mut body, 2, 1);
    field_bytes(&mut body, 3, &item);
    let mut op = Vec::new();
    field_varint(&mut op, 1, 3);
    field_bytes(&mut op, 3, &body);
    op
}

fn add_op(uri: &str) -> Vec<u8> {
    let mut item = Vec::new();
    field_bytes(&mut item, 1, uri.as_bytes());
    let mut body = Vec::new();
    field_bytes(&mut body, 2, &item);
    field_varint(&mut body, 4, 1);
    let mut op = Vec::new();
    field_varint(&mut op, 1, 2);
    field_bytes(&mut op, 2, &body);
    op
}

async fn send_changes(session: &Session, path: &str, revision: &[u8], op: &[u8]) -> anyhow::Result<()> {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let mut info = Vec::new();
    field_bytes(&mut info, 1, session.username().as_bytes());
    field_varint(&mut info, 2, now);
    let mut delta = Vec::new();
    field_bytes(&mut delta, 1, revision);
    field_bytes(&mut delta, 2, op);
    field_bytes(&mut delta, 4, &info);
    let mut changes = Vec::new();
    field_bytes(&mut changes, 1, revision);
    field_bytes(&mut changes, 2, &delta);
    let mut headers = HeaderMap::new();
    headers.insert("content-type", "application/x-protobuf".parse()?);
    session.spclient().request(&Method::POST, path, Some(headers), Some(&changes)).await?;
    Ok(())
}

pub async fn playlist_change(session: &Session, playlist_id: &str, uri: &str, add: bool) -> anyhow::Result<()> {
    use librespot::core::{SpotifyUri, spotify_id::SpotifyId};
    use librespot::metadata::{Metadata, Playlist};

    let id = SpotifyId::from_base62(playlist_id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let list = Playlist::get(session, &SpotifyUri::Playlist { user: None, id }).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let op = if add {
        add_op(uri)
    } else {
        let index = list
            .contents
            .items
            .iter()
            .position(|i| i.id.to_uri().is_ok_and(|u| u == uri))
            .ok_or_else(|| anyhow::anyhow!("track is not in this playlist"))?;
        rem_op(index, uri)
    };
    send_changes(session, &format!("/playlist/v2/playlist/{playlist_id}/changes"), &list.revision, &op).await
}

pub async fn rootlist_remove(session: &Session, uri: &str) -> anyhow::Result<()> {
    use librespot::protocol::playlist4_external::SelectedListContent;
    use protobuf::Message;

    let bytes = session.spclient().get_rootlist(0, Some(500)).await?;
    let list = SelectedListContent::parse_from_bytes(&bytes)?;
    let index = list
        .contents
        .get_or_default()
        .items
        .iter()
        .position(|i| i.uri() == uri)
        .ok_or_else(|| anyhow::anyhow!("playlist is not in your library"))?;
    let path = format!("/playlist/v2/user/{}/rootlist/changes", session.username());
    send_changes(session, &path, list.revision(), &rem_op(index, uri)).await
}

pub async fn artist_stats(session: &Session, id: &str) -> Option<(String, bool)> {
    let path = format!("/artistview/v1/artist/{id}?format=json");
    let raw = session.spclient().request_as_json(&Method::GET, &path, None, None).await.ok()?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    let accessory = v.pointer("/header/text/accessory").and_then(Value::as_str).unwrap_or_default();
    let listeners = accessory.strip_suffix(" monthly listeners").unwrap_or_default().to_owned();
    let verified = String::from_utf8_lossy(&raw).contains("\"verified\":true");
    Some((listeners, verified))
}
