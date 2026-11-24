use serde::{Deserialize, Serialize};

pub const DJ_URI: &str = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq";
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub uri: String,
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub image: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub eyebrow: Option<String>,
    pub title: String,
    pub items: Vec<Card>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hero {
    pub card: Card,
    pub eyebrow: String,
    pub headline: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Home {
    pub hero: Option<Hero>,
    pub shortcuts: Vec<Card>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Me {
    pub id: String,
    pub display_name: String,
    pub image: Option<String>,
    pub product: String,
    pub email: String,
    pub country: String,
    pub followers: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtistInfo {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub followers: Option<u64>,
    pub genres: Vec<String>,
    pub following: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NowPlaying {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub album: String,
    pub image_small: Option<String>,
    pub image_large: Option<String>,
    pub duration_ms: u32,
    pub explicit: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PlayerState {
    pub ready: bool,
    pub track: Option<NowPlaying>,
    pub is_playing: bool,
    pub position_ms: u32,
    pub shuffle: bool,
    pub repeat: String,
    pub volume: u8,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    pub player_ready: bool,
    pub player_error: Option<String>,
    pub player_status: String,
    pub library_ready: bool,
    pub library_connecting: bool,
}

pub fn plain(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut in_tag = false;
    for ch in raw.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    let decoded = [("&amp;", "&"), ("&quot;", "\""), ("&#x27;", "'"), ("&#39;", "'"), ("&lt;", "<"), ("&gt;", ">"), ("&#x2F;", "/"), ("&nbsp;", " ")]
        .iter()
        .fold(out, |text, (from, to)| text.replace(from, to));
    if decoded.trim() == "null" { String::new() } else { decoded.trim().to_owned() }
}

pub fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_owned()
}

pub fn image(v: &Value, min: u64) -> Option<String> {
    let images = v.get("images")?.as_array()?;
    let mut sized: Vec<(u64, &str)> = images
        .iter()
        .filter_map(|i| Some((i.get("width").and_then(Value::as_u64).unwrap_or(0), i.get("url")?.as_str()?)))
        .collect();
    sized.sort_by_key(|(w, _)| *w);
    sized
        .iter()
        .find(|(w, _)| *w >= min)
        .or_else(|| sized.last())
        .map(|(_, u)| (*u).to_owned())
}

pub fn joined_artists(v: &Value) -> String {
    v.get("artists")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|x| s(x, "name")).collect::<Vec<_>>().join(", "))
        .unwrap_or_default()
}

pub fn capitalise(word: &str) -> String {
    let mut c = word.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

pub fn release_label(album_type: &str, tracks: u64) -> String {
    match album_type {
        "single" if tracks >= 4 => "EP".into(),
        "" => "Album".into(),
        other => capitalise(other),
    }
}

impl Card {
    pub fn playlist(v: &Value) -> Self {
        let owner = v.get("owner").map(|o| s(o, "display_name")).unwrap_or_default();
        Self {
            id: s(v, "id"),
            uri: s(v, "uri"),
            kind: "playlist".into(),
            title: s(v, "name"),
            subtitle: format!("Playlist • {owner}"),
            image: image(v, 300),
            pinned: false,
        }
    }

    pub fn album(v: &Value) -> Self {
        let kind = release_label(&s(v, "album_type"), v.get("total_tracks").and_then(Value::as_u64).unwrap_or(0));
        Self {
            id: s(v, "id"),
            uri: s(v, "uri"),
            kind: "album".into(),
            title: s(v, "name"),
            subtitle: format!("{kind} • {}", joined_artists(v)),
            image: image(v, 300),
            pinned: false,
        }
    }

    pub fn artist(v: &Value) -> Self {
        Self {
            id: s(v, "id"),
            uri: s(v, "uri"),
            kind: "artist".into(),
            title: s(v, "name"),
            subtitle: "Artist".into(),
            image: image(v, 300),
            pinned: false,
        }
    }

    pub fn track(v: &Value) -> Self {
        let album = v.get("album").cloned().unwrap_or_default();
        Self {
            id: s(v, "id"),
            uri: s(v, "uri"),
            kind: "track".into(),
            title: s(v, "name"),
            subtitle: joined_artists(v),
            image: image(&album, 300),
            pinned: false,
        }
    }

    pub fn liked_songs(me: &Me, total: u64) -> Self {
        Self {
            id: "liked".into(),
            uri: format!("spotify:user:{}:collection", me.id),
            kind: "collection".into(),
            title: "Liked Songs".into(),
            subtitle: if total > 0 {
                format!("Playlist • {total} songs")
            } else {
                format!("Playlist • {}", me.display_name)
            },
            image: None,
            pinned: false,
        }
    }

    pub fn retag(mut self) -> Self {
        if self.uri == DJ_URI {
            self.kind = "dj".into();
            self.title = "DJ".into();
            self.subtitle = "Click to start listening".into();
        }
        self
    }

    pub fn your_episodes() -> Self {
        Self {
            id: "episodes".into(),
            uri: "spotify:user:collection:your-episodes".into(),
            kind: "episodes".into(),
            title: "Your Episodes".into(),
            subtitle: "Saved & downloaded episodes".into(),
            image: None,
            pinned: true,
        }
    }

    pub fn show(v: &Value) -> Self {
        Self {
            id: s(v, "id"),
            uri: s(v, "uri"),
            kind: "show".into(),
            title: s(v, "name"),
            subtitle: match s(v, "publisher") {
                p if p.is_empty() => "Podcast".into(),
                p => format!("Podcast • {p}"),
            },
            image: image(v, 300),
            pinned: false,
        }
    }

    pub fn episode(v: &Value) -> Self {
        let show = v.get("show").map(|sh| s(sh, "name")).unwrap_or_default();
        let date = Some(s(v, "release_date")).filter(|d| !d.starts_with("0000")).unwrap_or_default();
        Self {
            id: s(v, "id"),
            uri: s(v, "uri"),
            kind: "episode".into(),
            title: s(v, "name"),
            subtitle: [date, show].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" • "),
            image: image(v, 300),
            pinned: false,
        }
    }

    pub fn audiobook(v: &Value) -> Self {
        let authors = v
            .get("authors")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|x| s(x, "name")).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        Self {
            id: s(v, "id"),
            uri: s(v, "uri"),
            kind: "audiobook".into(),
            title: s(v, "name"),
            subtitle: authors,
            image: image(v, 300),
            pinned: false,
        }
    }

    pub fn pin(mut self) -> Self {
        self.pinned = true;
        self
    }
}
