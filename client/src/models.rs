use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct Section {
    pub eyebrow: Option<String>,
    pub title: String,
    pub items: Vec<Card>,
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hero {
    pub card: Card,
    pub headline: String,
    pub eyebrow: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Home {
    pub hero: Option<Hero>,
    pub shortcuts: Vec<Card>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Me {
    pub display_name: String,
    pub image: Option<String>,
    #[serde(default)]
    pub product: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub country: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtistInfo {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    pub followers: Option<u64>,
    pub genres: Vec<String>,
    pub following: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtistRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NowPlaying {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    #[serde(default)]
    pub album: String,
    pub image_small: Option<String>,
    pub image_large: Option<String>,
    pub duration_ms: u32,
    pub explicit: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayerState {
    pub ready: bool,
    pub track: Option<NowPlaying>,
    pub is_playing: bool,
    pub position_ms: u32,
    pub shuffle: bool,
    pub repeat: String,
    pub volume: u8,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub plugin_audio: Option<PluginAudio>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct PluginAudio {
    pub plugin: String,
    pub seq: u64,
    pub event: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    #[serde(default)]
    pub player_status: String,
    #[serde(default = "yes")]
    pub library_ready: bool,
    #[serde(default)]
    pub library_connecting: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackRow {
    pub id: String,
    pub uri: String,
    #[serde(default)]
    pub album_id: String,
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub album: String,
    pub image: Option<String>,
    pub duration: String,
    pub explicit: bool,
    pub added: String,
    pub added_by: String,
}

#[derive(Debug, Clone, Deserialize)]
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
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub owner_image: Option<String>,
    #[serde(default)]
    pub footer: Vec<String>,
    #[serde(default)]
    pub more: Vec<Card>,
    #[serde(default)]
    pub partial: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SavedState {
    pub liked: bool,
    pub playlists: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistOption {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    pub total: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtistPage {
    pub id: String,
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub followers: Option<u64>,
    pub following: bool,
    #[serde(default)]
    pub listeners: String,
    #[serde(default)]
    pub verified: bool,
    pub top: Vec<TrackRow>,
    pub albums: Vec<Card>,
    pub singles: Vec<Card>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackInfo {
    pub name: String,
    pub artists: Vec<ArtistRef>,
    pub album_id: String,
    pub album: String,
    pub release: String,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct Episode {
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

#[derive(Debug, Clone, Deserialize)]
pub struct Show {
    pub id: String,
    pub uri: String,
    pub title: String,
    pub publisher: String,
    pub description: String,
    pub image: Option<String>,
    #[serde(default)]
    pub saved: bool,
    pub episodes: Vec<Episode>,
}
