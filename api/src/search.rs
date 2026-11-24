use crate::collection::{TrackRow, track_row};
use crate::models::{Card, Section, image, s};
use crate::web::WebApi;
use futures::{StreamExt, future::join_all};
use serde::Serialize;
use serde_json::Value;

const PAGE: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub query: String,
    pub filter: String,
    pub top: Option<Card>,
    pub tracks: Vec<TrackRow>,
    pub sections: Vec<Section>,
    pub grid: Vec<Card>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tile {
    pub key: String,
    pub title: String,
    pub color: String,
    pub image: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Category {
    pub key: String,
    pub title: String,
    pub color: String,
    pub tracks: Vec<TrackRow>,
    pub sections: Vec<Section>,
}

struct TileDef {
    key: &'static str,
    title: &'static str,
    color: &'static str,
    query: &'static str,
    genre: Option<&'static str>,
}

const fn tile(key: &'static str, title: &'static str, color: &'static str, query: &'static str, genre: Option<&'static str>) -> TileDef {
    TileDef { key, title, color, query, genre }
}

const TILES: &[TileDef] = &[
    tile("made-for-you", "Made For You", "#1e3264", "", None),
    tile("new-releases", "New Releases", "#e8115b", "tag:new", None),
    tile("podcasts", "Podcasts", "#006450", "podcast", None),
    tile("hip-hop", "Hip-Hop", "#bc5900", "hip hop", Some("hip hop")),
    tile("pop", "Pop", "#8d67ab", "pop", Some("pop")),
    tile("rock", "Rock", "#e91429", "rock", Some("rock")),
    tile("dance", "Dance/Electronic", "#d84000", "edm", Some("edm")),
    tile("indie", "Indie", "#e1118c", "indie", Some("indie")),
    tile("rnb", "R&B", "#dc148c", "r&b", Some("r&b")),
    tile("latin", "Latin", "#e1118b", "latin", Some("latin")),
    tile("country", "Country", "#d84000", "country", Some("country")),
    tile("metal", "Metal", "#e91429", "metal", Some("metal")),
    tile("jazz", "Jazz", "#477d95", "jazz", Some("jazz")),
    tile("classical", "Classical", "#7d4b32", "classical", Some("classical")),
    tile("kpop", "K-pop", "#148a08", "k-pop", Some("k-pop")),
    tile("phonk", "Phonk", "#503750", "phonk", Some("phonk")),
    tile("anime", "Anime", "#e8115b", "anime", Some("anime")),
    tile("chill", "Chill", "#b06239", "chill", None),
    tile("workout", "Workout", "#777777", "workout", None),
    tile("party", "Party", "#537aa1", "party", None),
    tile("focus", "Focus", "#503750", "focus", None),
    tile("sleep", "Sleep", "#1e3264", "sleep", None),
    tile("gaming", "Gaming", "#e8115b", "gaming", None),
    tile("throwback", "Throwback", "#bc5900", "throwback", None),
];

fn items(v: &Value, key: &str) -> Vec<Value> {
    v.pointer(&format!("/{key}/items"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|i| !i.is_null()).cloned().collect())
        .unwrap_or_default()
}

fn encode(query: &str) -> String {
    url::form_urlencoded::byte_serialize(query.as_bytes()).collect()
}

fn normalise(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

pub async fn popular_shows(api: &WebApi) -> Vec<Card> {
    let p = pages(api, "podcast", "show", 2).await;
    collect(&p, "shows").iter().map(Card::show).collect()
}

async fn pages(api: &WebApi, query: &str, kinds: &str, count: usize) -> Vec<Value> {
    let q = encode(query);
    join_all((0..count).map(|page| {
        let path = format!("/search?q={q}&type={kinds}&limit={PAGE}&offset={}", page * PAGE);
        async move { api.get::<Value>(&path).await.unwrap_or(Value::Null) }
    }))
    .await
}

fn collect(pages: &[Value], key: &str) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    pages
        .iter()
        .flat_map(|p| items(p, key))
        .filter(|v| seen.insert(s(v, "id")))
        .collect()
}

fn quality(query: &str, name: &str) -> u8 {
    let (q, n) = (normalise(query), normalise(name));
    if q.is_empty() || n.is_empty() {
        return 0;
    }
    if n == q {
        return 3;
    }
    if n.starts_with(&q) {
        return 2;
    }
    let words = query.to_lowercase();
    let lower = name.to_lowercase();
    if lower.split(|c: char| !c.is_alphanumeric()).any(|w| !w.is_empty() && words.split_whitespace().any(|q| q == w)) {
        return 1;
    }
    0
}

fn pick_top(query: &str, artists: &[Value], tracks: &[Value], albums: &[Value], playlists: &[Value], shows: &[Value]) -> Option<Card> {
    let popularity = |v: &Value| v.get("popularity").and_then(Value::as_u64).unwrap_or(0);
    let candidates: [(Option<&Value>, fn(&Value) -> Card); 5] = [
        (artists.first(), Card::artist),
        (tracks.first(), Card::track),
        (albums.first(), Card::album),
        (playlists.first(), Card::playlist),
        (shows.first(), Card::show),
    ];
    let mut best: Option<(u8, usize)> = None;
    for (i, (item, _)) in candidates.iter().enumerate() {
        let Some(item) = item else { continue };
        let score = quality(query, &s(item, "name"));
        if best.is_none_or(|(top, _)| score > top) {
            best = Some((score, i));
        }
    }
    let (score, mut index) = best?;
    if index == 0 && score < 3
        && let (Some(a), Some(t)) = (artists.first(), tracks.first())
            && quality(query, &s(t, "name")) == score && popularity(t) > popularity(a) + 5 {
                index = 1;
            }
    let (item, build) = candidates[index];
    item.map(build)
}

fn section(title: &str, cards: Vec<Card>) -> Option<Section> {
    (!cards.is_empty()).then(|| Section { eyebrow: None, title: title.into(), items: cards, kind: String::new() })
}

pub async fn search(api: &WebApi, query: &str, filter: &str) -> anyhow::Result<SearchResults> {
    let mut out = SearchResults {
        query: query.to_owned(),
        filter: filter.to_owned(),
        top: None,
        tracks: Vec::new(),
        sections: Vec::new(),
        grid: Vec::new(),
    };

    match filter {
        "songs" => {
            let p = pages(api, query, "track", 5).await;
            out.tracks = collect(&p, "tracks").iter().filter_map(|t| track_row(t, "", String::new())).collect();
        }
        "artists" => out.grid = collect(&pages(api, query, "artist", 4).await, "artists").iter().map(Card::artist).collect(),
        "albums" => out.grid = collect(&pages(api, query, "album", 4).await, "albums").iter().map(Card::album).collect(),
        "playlists" => out.grid = collect(&pages(api, query, "playlist", 4).await, "playlists").iter().map(Card::playlist).collect(),
        "podcasts" => {
            let p = pages(api, query, "show,episode", 3).await;
            out.grid = collect(&p, "shows").iter().map(Card::show).collect();
            out.sections.extend(section("Episodes", collect(&p, "episodes").iter().map(Card::episode).collect()));
        }
        _ => {
            let first: Value = api
                .get(&format!("/search?q={}&type=track,artist,album,playlist,show,episode&limit={PAGE}", encode(query)))
                .await?;
            let (tracks, artists, albums, playlists) =
                (items(&first, "tracks"), items(&first, "artists"), items(&first, "albums"), items(&first, "playlists"));
            out.top = pick_top(query, &artists, &tracks, &albums, &playlists, &items(&first, "shows"));
            out.tracks = tracks.iter().filter_map(|t| track_row(t, "", String::new())).collect();
            out.sections.extend(section("Artists", artists.iter().map(Card::artist).collect()));
            out.sections.extend(section("Albums", albums.iter().map(Card::album).collect()));
            out.sections.extend(section("Playlists", playlists.iter().map(Card::playlist).collect()));
            out.sections.extend(section("Podcasts", items(&first, "shows").iter().map(Card::show).collect()));
            out.sections.extend(section("Episodes", items(&first, "episodes").iter().map(Card::episode).collect()));
        }
    }
    Ok(out)
}

async fn tile_image(api: &WebApi, def: &TileDef) -> Option<String> {
    let (query, kind) = match (def.key, def.genre) {
        ("made-for-you", _) => return None,
        ("new-releases", _) => ("tag:new".to_owned(), "album"),
        ("podcasts", _) => ("podcast".to_owned(), "show"),
        (_, Some(genre)) => (format!("genre:\"{genre}\""), "artist"),
        _ => (def.query.to_owned(), "track"),
    };
    let found = first_image(api, &query, kind).await;
    if found.is_some() || kind != "artist" {
        return found;
    }
    first_image(api, def.query, "track").await
}

async fn first_image(api: &WebApi, query: &str, kind: &str) -> Option<String> {
    let v: Value = api
        .get(&format!("/search?q={}&type={kind}&limit=3", encode(query)))
        .await
        .ok()?;
    items(&v, &format!("{kind}s")).iter().find_map(|item| match kind {
        "track" => item.get("album").and_then(|a| image(a, 300)),
        _ => image(item, 300),
    })
}

pub async fn tiles(api: &WebApi) -> Vec<Tile> {
    let images: Vec<Option<String>> = futures::stream::iter(0..TILES.len())
        .map(|i| tile_image(api, &TILES[i]))
        .buffered(4)
        .collect()
        .await;
    TILES
        .iter()
        .zip(images)
        .map(|(def, image)| Tile { key: def.key.into(), title: def.title.into(), color: def.color.into(), image })
        .collect()
}

pub async fn category(api: &WebApi, key: &str, made_for: Vec<Card>) -> anyhow::Result<Category> {
    let def = TILES.iter().find(|t| t.key == key).ok_or_else(|| anyhow::anyhow!("unknown category"))?;
    let mut out = Category {
        key: key.into(),
        title: def.title.into(),
        color: def.color.into(),
        tracks: Vec::new(),
        sections: Vec::new(),
    };

    match key {
        "made-for-you" => out.sections.extend(section("Made For You", made_for)),
        "new-releases" => {
            let albums = collect(&pages(api, "tag:new", "album", 4).await, "albums");
            let (singles, full): (Vec<Value>, Vec<Value>) =
                albums.into_iter().partition(|a| s(a, "album_type") == "single");
            out.sections.extend(section("New albums", full.iter().map(Card::album).collect()));
            out.sections.extend(section("New singles", singles.iter().map(Card::album).collect()));
        }
        "podcasts" => {
            let p = pages(api, "podcast", "show", 4).await;
            out.sections.extend(section("Popular podcasts", collect(&p, "shows").iter().map(Card::show).collect()));
        }
        _ => {
            let filter = def.genre.map(|g| format!("genre:\"{g}\"")).unwrap_or_else(|| def.query.to_owned());
            let (artists, tracks, playlists) = tokio::join!(
                pages(api, &filter, "artist", 2),
                pages(api, &filter, "track", 3),
                pages(api, def.query, "playlist", 3),
            );
            let mut artists = collect(&artists, "artists");
            artists.sort_by_key(|a| std::cmp::Reverse(a.get("popularity").and_then(Value::as_u64).unwrap_or(0)));
            let mut tracks = collect(&tracks, "tracks");
            tracks.sort_by_key(|t| std::cmp::Reverse(t.get("popularity").and_then(Value::as_u64).unwrap_or(0)));
            out.tracks = tracks.iter().take(10).filter_map(|t| track_row(t, "", String::new())).collect();
            out.sections.extend(section("Popular artists", artists.iter().map(Card::artist).collect()));
            out.sections.extend(section("Playlists", collect(&playlists, "playlists").iter().map(Card::playlist).collect()));
            let mut seen = std::collections::HashSet::new();
            let albums: Vec<Card> = tracks
                .iter()
                .filter_map(|t| t.get("album"))
                .filter(|a| seen.insert(s(a, "id")))
                .map(Card::album)
                .collect();
            out.sections.extend(section("Albums", albums));
        }
    }
    Ok(out)
}
