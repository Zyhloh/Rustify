use crate::models::{Card, Hero, Home, Me, Section, image, s};
use crate::web::WebApi;
use futures::future::join_all;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

fn items(v: &Value) -> Vec<Value> {
    v.get("items").and_then(Value::as_array).cloned().unwrap_or_default()
}

async fn opt(api: &WebApi, path: &str) -> Value {
    match api.get::<Value>(path).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("{e}");
            Value::Null
        }
    }
}

pub async fn me(api: &WebApi) -> anyhow::Result<Me> {
    let v: Value = api.get("/me").await?;
    Ok(Me {
        id: s(&v, "id"),
        display_name: s(&v, "display_name"),
        image: image(&v, 64),
        product: s(&v, "product"),
        email: s(&v, "email"),
        country: s(&v, "country"),
        followers: v.pointer("/followers/total").and_then(Value::as_u64).unwrap_or(0),
    })
}

pub async fn library(
    api: &WebApi,
    session: Option<&librespot::core::session::Session>,
    me: &Me,
    recents: HashMap<String, i64>,
    pins: Vec<String>,
    episode_count: u64,
    root: Vec<crate::native::RootItem>,
) -> Vec<Card> {
    let (playlists, albums, artists, liked, shows) = tokio::join!(
        opt(api, "/me/playlists?limit=50"),
        opt(api, "/me/albums?limit=50"),
        opt(api, "/me/following?type=artist&limit=50"),
        opt(api, "/me/tracks?limit=1"),
        opt(api, "/me/shows?limit=50"),
    );

    let mut cards: Vec<Card> = items(&playlists).iter().map(Card::playlist).collect();

    let known: HashSet<String> = cards.iter().map(|c| c.uri.clone()).collect();
    let hidden: Vec<&crate::native::RootItem> = root.iter().filter(|r| !known.contains(&r.uri)).collect();
    let filled = join_all(hidden.iter().filter(|r| !r.name.is_empty()).map(|r| async move {
        let fallback = || Card {
            id: r.uri.rsplit(':').next().unwrap_or_default().to_owned(),
            uri: r.uri.clone(),
            kind: "playlist".into(),
            title: r.name.clone(),
            subtitle: match r.owner.as_str() {
                "spotify" => "Playlist • Spotify".into(),
                owner if owner == me.id => format!("Playlist • {}", me.display_name),
                owner => format!("Playlist • {owner}"),
            },
            image: r.image.clone(),
            pinned: false,
        };
        let foreign = r.owner != "spotify" && r.owner != me.id;
        if r.image.is_some() && !foreign {
            return fallback();
        }
        let mut card = match decorate(api, &r.uri).await {
            Some(card) if card.image.is_some() => return card,
            Some(card) => card,
            None => fallback(),
        };
        if card.image.is_none() {
            card.image = r.image.clone();
        }
        if let (Some(session), None) = (session, &card.image) {
            card.image = crate::collection::playlist_cover_native(session, &card.id).await;
        }
        card
    }))
    .await;
    cards.extend(filled);
    for card in cards.iter_mut() {
        if root.iter().any(|r| r.uri == card.uri && r.made_for) {
            card.subtitle = format!("Playlist • Made for {}", me.display_name);
        }
    }
    let web_failed = albums.is_null() || artists.is_null() || shows.is_null() || liked.is_null();
    let native = match (session, web_failed) {
        (Some(session), true) => Some(crate::native::library(session).await),
        _ => None,
    };
    match native.as_ref() {
        Some(n) if albums.is_null() => cards.extend(n.albums.iter().cloned()),
        _ => cards.extend(items(&albums).iter().filter_map(|i| i.get("album")).map(Card::album)),
    }
    match native.as_ref() {
        Some(n) if shows.is_null() => cards.extend(n.shows.iter().cloned()),
        _ => cards.extend(items(&shows).iter().filter_map(|i| i.get("show")).map(Card::show)),
    }
    match native.as_ref() {
        Some(n) if artists.is_null() => cards.extend(n.artists.iter().cloned()),
        _ => cards.extend(artists.get("artists").map(items).unwrap_or_default().iter().map(Card::artist)),
    }

    let added: HashMap<&str, i64> = root.iter().map(|r| (r.uri.as_str(), r.added)).collect();
    cards.sort_by_key(|c| {
        let last = recents.get(&c.uri).copied().unwrap_or(0).max(added.get(c.uri.as_str()).copied().unwrap_or(0));
        if last > 0 { (0i64, -last) } else { (1, 0) }
    });

    let pin_set: std::collections::HashSet<String> = pins.iter().cloned().collect();
    cards.retain(|c| !pin_set.contains(&c.uri));

    let pinned = join_all(pins.iter().map(|uri| decorate(api, uri))).await;

    let total = liked
        .get("total")
        .and_then(Value::as_u64)
        .or_else(|| native.as_ref().map(|n| n.liked))
        .unwrap_or(0);
    let mut out = vec![Card::liked_songs(me, total).pin()];
    let pinned: Vec<Card> = pinned
        .into_iter()
        .flatten()
        .map(|c| if c.title == "Your Episodes" { Card::your_episodes() } else { c.pin() })
        .collect();
    if episode_count > 0 && !pinned.iter().any(|c| c.kind == "episodes") {
        out.push(Card::your_episodes());
    }
    out.extend(pinned);
    out.extend(cards);
    out
}

async fn decorate(api: &WebApi, uri: &str) -> Option<Card> {
    let mut parts = uri.rsplitn(3, ':');
    let id = parts.next()?.to_owned();
    let kind = parts.next()?;
    match kind {
        "playlist" => api
            .get::<Value>(&format!("/playlists/{id}?fields=id,uri,name,images,owner(display_name)"))
            .await
            .ok()
            .map(|v| Card::playlist(&v)),
        "album" => api.get::<Value>(&format!("/albums/{id}")).await.ok().map(|v| Card::album(&v)),
        "artist" => api.get::<Value>(&format!("/artists/{id}")).await.ok().map(|v| Card::artist(&v)),
        "show" => api.get::<Value>(&format!("/shows/{id}")).await.ok().map(|v| Card::show(&v)),
        _ => None,
    }
}

async fn native_recents(api: &WebApi, session: &librespot::core::session::Session, me: &Me) -> Vec<Card> {
    let mut played: Vec<(String, i64)> = crate::native::recently_played(session).await.into_iter().collect();
    played.sort_by_key(|item| std::cmp::Reverse(item.1));
    join_all(played.iter().filter(|(uri, _)| !uri.ends_with(":collection")).take(12).map(|(uri, _)| async move {
        let mut card = match decorate_native(session, uri).await {
            Some(card) => card,
            None => decorate(api, uri).await?,
        };
        if card.kind == "playlist" {
            if card.subtitle == format!("Playlist • {}", me.id) {
                card.subtitle = format!("Playlist • {}", me.display_name);
            }
            if card.image.is_none() {
                card.image = crate::collection::playlist_cover_native(session, &card.id).await;
            }
        }
        Some(card)
    }))
    .await
    .into_iter()
    .flatten()
    .collect()
}

async fn decorate_native(session: &librespot::core::session::Session, uri: &str) -> Option<Card> {
    use librespot::core::SpotifyUri;
    use librespot::metadata::{Album, Artist, Metadata, Playlist, Show};
    let parsed = SpotifyUri::from_uri(uri).ok()?;
    let id = uri.rsplit(':').next()?.to_owned();
    let card = |kind: &str, title: String, subtitle: String, image: Option<String>| Card {
        id: id.clone(),
        uri: uri.to_owned(),
        kind: kind.into(),
        title,
        subtitle,
        image,
        pinned: false,
    };
    match &parsed {
        SpotifyUri::Playlist { .. } => {
            let list = Playlist::get(session, &parsed).await.ok()?;
            let attrs = &list.attributes;
            let image = ["large", "default", "small"]
                .iter()
                .find_map(|want| attrs.picture_sizes.iter().find(|p| p.target_name == *want).map(|p| p.url.clone()))
                .or_else(|| attrs.picture_sizes.first().map(|p| p.url.clone()))
                .or_else(|| ["image_url", "image"].iter().find_map(|k| attrs.format_attributes.get(*k)).and_then(|raw| crate::extras::spotify_image(raw)))
                .filter(|u| !u.is_empty());
            let owner = match &list.id {
                SpotifyUri::Playlist { user: Some(user), .. } if user != "spotify" => user.clone(),
                _ => "Spotify".into(),
            };
            Some(card("playlist", attrs.name.clone(), format!("Playlist • {owner}"), image))
        }
        SpotifyUri::Artist { .. } => {
            let artist = Artist::get(session, &parsed).await.ok()?;
            let image = crate::collection::cover_url(&artist.portraits, 300).or_else(|| crate::collection::cover_url(&artist.portrait_group, 300));
            Some(card("artist", artist.name.clone(), "Artist".into(), image))
        }
        SpotifyUri::Album { .. } => {
            let album = Album::get(session, &parsed).await.ok()?;
            let by = album.artists.first().map(|a| a.name.clone()).unwrap_or_default();
            let label = crate::models::release_label(if album.tracks().count() <= 6 { "single" } else { "album" }, album.tracks().count() as u64);
            Some(card("album", album.name.clone(), format!("{label} • {by}"), crate::collection::cover_url(&album.covers, 300)))
        }
        SpotifyUri::Show { .. } => {
            let show = Show::get(session, &parsed).await.ok()?;
            Some(card("show", show.name.clone(), format!("Podcast • {}", show.publisher), crate::collection::cover_url(&show.covers, 300)))
        }
        _ => None,
    }
}

async fn recent_contexts(api: &WebApi, recent: &Value) -> Vec<Card> {
    let mut seen = HashSet::new();
    let mut pending = Vec::new();

    for item in items(recent) {
        let track = item.get("track").cloned().unwrap_or_default();
        let context = item.get("context").filter(|c| !c.is_null());
        let (kind, uri) = match context {
            Some(c) => (s(c, "type"), s(c, "uri")),
            None => {
                let album = track.get("album").cloned().unwrap_or_default();
                ("album".into(), s(&album, "uri"))
            }
        };
        if uri.is_empty() || !seen.insert(uri.clone()) {
            continue;
        }
        pending.push((kind, uri, track));
        if pending.len() >= 12 {
            break;
        }
    }

    let fetches = pending.into_iter().map(|(kind, uri, track)| async move {
        let id = uri.rsplit(':').next().unwrap_or_default().to_owned();
        match kind.as_str() {
            "playlist" => api
                .get::<Value>(&format!("/playlists/{id}?fields=id,uri,name,images,owner(display_name)"))
                .await
                .ok()
                .map(|v| Card::playlist(&v)),
            "artist" => api
                .get::<Value>(&format!("/artists/{id}"))
                .await
                .ok()
                .map(|v| Card::artist(&v)),
            "album" => {
                let album = track.get("album").cloned().unwrap_or_default();
                if s(&album, "uri") == uri {
                    Some(Card::album(&album))
                } else {
                    api.get::<Value>(&format!("/albums/{id}")).await.ok().map(|v| Card::album(&v))
                }
            }
            _ => None,
        }
    });

    join_all(fetches).await.into_iter().flatten().collect()
}

async fn hero(session: Option<&librespot::core::session::Session>, top_artists: &[Value]) -> Option<Hero> {
    let session = session?;
    let fetches = top_artists.iter().take(6).map(|artist| async move {
        let id = s(artist, "id");
        crate::artist::releases(session, &id, 2).await.into_iter().next()
    });

    let newest = join_all(fetches).await.into_iter().flatten().max_by(|a, b| a.date.cmp(&b.date))?;
    let kind = if newest.single { "single" } else { "album" };
    let mut card = newest.card;
    card.image = newest.large.or(card.image);
    card.subtitle = newest.artist.clone();

    Some(Hero {
        headline: format!("The new {kind} from {} is out.", newest.artist),
        eyebrow: format!("{} • New release", card.title),
        card,
    })
}

pub async fn home(api: &WebApi, me: &Me, session: Option<librespot::core::session::Session>) -> Home {
    let (recent, top_artists, top_tracks, playlists, albums, liked, shows, episodes, audiobooks) = tokio::join!(
        opt(api, "/me/player/recently-played?limit=50"),
        opt(api, "/me/top/artists?limit=12&time_range=short_term"),
        opt(api, "/me/top/tracks?limit=12&time_range=short_term"),
        opt(api, "/me/playlists?limit=20"),
        opt(api, "/me/albums?limit=12"),
        opt(api, "/me/tracks?limit=1"),
        opt(api, "/me/shows?limit=20"),
        opt(api, "/me/episodes?limit=20"),
        opt(api, "/me/audiobooks?limit=20"),
    );

    let top_artists = items(&top_artists);
    let (recents, hero) = tokio::join!(
        async {
            match &session {
                Some(session) => {
                    let native = native_recents(api, session, me).await;
                    if native.is_empty() { recent_contexts(api, &recent).await } else { native }
                }
                None => recent_contexts(api, &recent).await,
            }
        },
        hero(session.as_ref(), &top_artists)
    );

    let liked_total = liked.get("total").and_then(Value::as_u64).unwrap_or(0);
    let mut recents = recents;
    let mut seen_recent: HashSet<String> = HashSet::new();
    for card in recents.iter_mut() {
        if card.kind == "playlist" && card.title == "Liked Songs" {
            *card = Card::liked_songs(me, liked_total);
        }
    }
    recents.retain(|card| seen_recent.insert(card.uri.clone()));
    let playlist_cards: Vec<Card> = items(&playlists).iter().map(Card::playlist).collect();

    let mut shortcuts = vec![Card::liked_songs(me, liked_total)];
    let mut seen: HashSet<String> = HashSet::new();
    for card in recents.iter().chain(playlist_cards.iter()) {
        if shortcuts.len() >= 8 {
            break;
        }
        if card.kind != "collection" && !card.uri.ends_with(":collection") && seen.insert(card.uri.clone()) {
            shortcuts.push(card.clone());
        }
    }

    let mut sections = Vec::new();
    let mut push = |kind: &str, eyebrow: Option<&str>, title: String, items: Vec<Card>| {
        if !items.is_empty() {
            sections.push(Section {
                eyebrow: eyebrow.map(str::to_owned),
                title,
                items,
                kind: kind.into(),
            });
        }
    };

    push("music", None, "Recently played".into(), recents);
    push("music", None, "Your top artists".into(), top_artists.iter().map(Card::artist).collect());
    push("music", Some("Made For"), me.display_name.clone(), playlist_cards);
    push("music", None, "On repeat".into(), items(&top_tracks).iter().map(Card::track).collect());
    push(
        "music",
        None,
        "Jump back in".into(),
        items(&albums).iter().filter_map(|i| i.get("album")).map(Card::album).collect(),
    );
    push("podcast", None, "Your shows".into(), items(&shows).iter().filter_map(|i| i.get("show")).map(Card::show).collect());
    push(
        "podcast",
        None,
        "Saved episodes".into(),
        items(&episodes)
            .iter()
            .filter_map(|i| i.get("episode"))
            .map(Card::episode)
            .filter(|c| !c.id.is_empty() && !c.title.is_empty())
            .collect(),
    );
    push("audiobook", None, "Your audiobooks".into(), items(&audiobooks).iter().map(Card::audiobook).collect());
    if !sections.iter().any(|s| s.title == "Your shows") {
        let popular = crate::search::popular_shows(api).await;
        if !popular.is_empty() {
            sections.push(Section { eyebrow: None, title: "Popular podcasts".into(), items: popular, kind: "podcast".into() });
        }
    }

    let shortcuts = shortcuts.into_iter().map(Card::retag).collect();
    let sections = sections
        .into_iter()
        .map(|mut section| {
            section.items = section.items.into_iter().map(Card::retag).collect();
            section
        })
        .collect();
    Home { hero, shortcuts, sections }
}

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub card: Card,
    pub date: String,
    pub kind: String,
    pub detail: String,
}

pub async fn whats_new(api: &WebApi, session: Option<librespot::core::session::Session>) -> Vec<Release> {
    use futures::stream::{self, StreamExt};

    let (following, shows) = tokio::join!(opt(api, "/me/following?type=artist&limit=40"), opt(api, "/me/shows?limit=20"));
    let mut artists: Vec<String> = following.get("artists").map(items).unwrap_or_default().iter().map(|a| s(a, "id")).collect();
    let mut show_ids: Vec<String> = items(&shows).iter().filter_map(|i| i.get("show")).map(|v| s(v, "id")).collect();
    if let Some(session) = &session {
        let tail = |uri: String| uri.rsplit(':').next().unwrap_or_default().to_owned();
        if following.is_null() {
            artists = crate::native::collection_set(session, "artist").await.into_iter().map(tail).take(40).collect();
        }
        if shows.is_null() {
            show_ids = crate::native::collection_set(session, "show").await.into_iter().map(tail).take(20).collect();
        }
    }

    let music: Vec<crate::artist::Release> = match &session {
        Some(session) => stream::iter(artists.into_iter().filter(|id| !id.is_empty()))
            .map(|id| async move { crate::artist::releases(session, &id, 2).await })
            .buffer_unordered(6)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .flatten()
            .collect(),
        None => Vec::new(),
    };
    let podcasts = stream::iter(show_ids.into_iter().filter(|id| !id.is_empty()))
        .map(|id| async move { opt(api, &format!("/shows/{id}/episodes?limit=3")).await })
        .buffer_unordered(6)
        .collect::<Vec<_>>()
        .await;

    let cutoff = days_ago(60);
    let mut seen = HashSet::new();
    let mut out: Vec<Release> = Vec::new();
    for r in music {
        if r.date < cutoff || !seen.insert(r.card.id.clone()) {
            continue;
        }
        let mut card = r.card;
        card.subtitle = r.artist;
        out.push(Release {
            card,
            date: r.date,
            kind: "music".into(),
            detail: r.label,
        });
    }
    for v in &podcasts {
        for episode in items(v) {
            let date = s(&episode, "release_date");
            if date.len() < 10 || date < cutoff || !seen.insert(s(&episode, "id")) {
                continue;
            }
            out.push(Release {
                card: Card::episode(&episode),
                date,
                kind: "podcast".into(),
                detail: "Episode".into(),
            });
        }
    }
    out.sort_by(|a, b| b.date.cmp(&a.date));
    out.truncate(60);
    out
}

fn days_ago(days: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let z = now / 86_400 - days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}
