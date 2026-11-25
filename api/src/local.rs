use crate::config::Config;
use lofty::{
    file::{AudioFile, TaggedFileExt},
    tag::Accessor,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::RwLock,
    time::UNIX_EPOCH,
};

const EXTENSIONS: [&str; 7] = ["mp3", "m4a", "aac", "flac", "ogg", "opus", "wav"];
const MAX_FILES: usize = 20_000;
const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalTrack {
    pub id: String,
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
    #[serde(skip_serializing)]
    pub path: PathBuf,
    #[serde(skip)]
    pub token: String,
    #[serde(skip_serializing, default)]
    pub modified: u64,
    #[serde(skip_serializing, default)]
    pub size: u64,
}

#[derive(Default)]
pub struct Library {
    settings: RwLock<Settings>,
    tracks: RwLock<Vec<LocalTrack>>,
    by_token: RwLock<HashMap<String, PathBuf>>,
    scanned: std::sync::Mutex<Option<std::time::Instant>>,
}

fn settings_path() -> PathBuf {
    Config::data_dir().join("local.json")
}

fn index_path() -> PathBuf {
    Config::data_dir().join("local-index.json")
}

fn encode(part: &str) -> String {
    let mut out = String::new();
    for b in part.as_bytes() {
        match *b {
            b' ' => out.push('+'),
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'*' => out.push(*b as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn decode(part: &str) -> String {
    let bytes = part.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn local_uri(artist: &str, album: &str, title: &str, duration_ms: u32) -> String {
    format!("spotify:local:{}:{}:{}:{}", encode(artist), encode(album), encode(title), duration_ms / 1000)
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalRef {
    pub artist: String,
    pub album: String,
    pub title: String,
    pub seconds: u32,
}

pub fn parse_uri(uri: &str) -> Option<LocalRef> {
    let rest = uri.strip_prefix("spotify:local:")?;
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(LocalRef { artist: decode(parts[0]), album: decode(parts[1]), title: decode(parts[2]), seconds: parts[3].parse().unwrap_or(0) })
}

fn norm(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

fn read(path: &Path, modified: u64, size: u64) -> Option<LocalTrack> {
    let tagged = lofty::read_from_path(path).ok();
    let duration_ms = tagged.as_ref().map(|t| t.properties().duration().as_millis() as u32).unwrap_or(0);
    let tag = tagged.as_ref().and_then(|t| t.primary_tag().or_else(|| t.first_tag()));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Unknown").to_owned();
    let clean = |s: Option<std::borrow::Cow<'_, str>>| s.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let title = clean(tag.and_then(|t| t.title())).unwrap_or(stem);
    let artist = clean(tag.and_then(|t| t.artist())).unwrap_or_default();
    let album = clean(tag.and_then(|t| t.album())).unwrap_or_default();
    let id = hash(&path.to_string_lossy());
    Some(LocalTrack {
        uri: local_uri(&artist, &album, &title, duration_ms),
        id,
        title,
        artist,
        album,
        duration_ms,
        path: path.to_path_buf(),
        token: String::new(),
        modified,
        size,
    })
}

fn walk(root: &Path, out: &mut Vec<(PathBuf, u64, u64)>) {
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if out.len() >= MAX_FILES {
                return;
            }
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.file_type().is_symlink() {
                continue;
            }
            let path = entry.path();
            if meta.is_dir() {
                if depth < MAX_DEPTH && !entry.file_name().to_string_lossy().starts_with('.') {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
            if EXTENSIONS.contains(&ext.as_str()) {
                let modified = meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                out.push((path, modified, meta.len()));
            }
        }
    }
}

impl Library {
    pub fn load() -> Self {
        let settings: Settings = std::fs::read(settings_path()).ok().and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or_default();
        let cached: Vec<LocalTrack> = std::fs::read(index_path()).ok().and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or_default();
        let lib = Self { settings: RwLock::new(settings), ..Default::default() };
        lib.install(cached);
        lib
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_settings(&self, mut next: Settings) -> anyhow::Result<()> {
        next.folders.retain(|f| Path::new(f).is_absolute());
        next.folders.sort();
        next.folders.dedup();
        next.folders.truncate(20);
        std::fs::write(settings_path(), serde_json::to_vec_pretty(&next)?)?;
        if let Ok(mut s) = self.settings.write() {
            *s = next;
        }
        Ok(())
    }

    fn install(&self, mut tracks: Vec<LocalTrack>) {
        let mut map = HashMap::new();
        for t in &mut tracks {
            t.token = token();
            map.insert(t.token.clone(), t.path.clone());
        }
        tracks.sort_by(|a, b| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()).then(a.album.cmp(&b.album)).then(a.title.cmp(&b.title)));
        if let Ok(mut t) = self.tracks.write() {
            *t = tracks;
        }
        if let Ok(mut m) = self.by_token.write() {
            *m = map;
        }
    }

    pub fn stale(&self) -> bool {
        self.settings().enabled && self.scanned.lock().map(|s| s.is_none_or(|t| t.elapsed().as_secs() >= 30)).unwrap_or(false)
    }

    pub fn scan(&self) -> usize {
        if let Ok(mut s) = self.scanned.lock() {
            *s = Some(std::time::Instant::now());
        }
        let settings = self.settings();
        let previous: HashMap<PathBuf, LocalTrack> = self.tracks.read().map(|t| t.iter().map(|x| (x.path.clone(), x.clone())).collect()).unwrap_or_default();
        let mut files = Vec::new();
        if settings.enabled {
            for folder in &settings.folders {
                walk(Path::new(folder), &mut files);
            }
        }
        let tracks: Vec<LocalTrack> = files
            .into_iter()
            .filter_map(|(path, modified, size)| match previous.get(&path) {
                Some(old) if old.modified == modified && old.size == size => Some(old.clone()),
                _ => read(&path, modified, size),
            })
            .collect();
        let count = tracks.len();
        if let Ok(raw) = serde_json::to_vec(&tracks) {
            let _ = std::fs::write(index_path(), raw);
        }
        self.install(tracks);
        count
    }

    pub fn tracks(&self) -> Vec<LocalTrack> {
        if !self.settings().enabled {
            return Vec::new();
        }
        self.tracks.read().map(|t| t.clone()).unwrap_or_default()
    }

    pub fn path_for_token(&self, token: &str) -> Option<PathBuf> {
        if token.len() != 64 || !self.settings().enabled {
            return None;
        }
        self.by_token.read().ok()?.get(token).cloned()
    }

    pub fn matching(&self, uri: &str) -> Option<LocalTrack> {
        if !self.settings().enabled {
            return None;
        }
        let wanted = parse_uri(uri)?;
        let tracks = self.tracks.read().ok()?;
        if let Some(exact) = tracks.iter().find(|t| t.uri == uri) {
            return Some(exact.clone());
        }
        let (title, artist) = (norm(&wanted.title), norm(&wanted.artist));
        tracks
            .iter()
            .find(|t| norm(&t.title) == title && (artist.is_empty() || norm(&t.artist) == artist) && (wanted.seconds == 0 || (t.duration_ms / 1000).abs_diff(wanted.seconds) <= 2))
            .cloned()
    }

    pub fn present(&self, uri: &str) -> Option<LocalTrack> {
        self.matching(uri).filter(|t| t.path.is_file())
    }
}

pub fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("mp3") => "audio/mpeg",
        Some("m4a") | Some("aac") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("ogg") | Some("opus") => "audio/ogg",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_uris_round_trip() {
        let uri = local_uri("Juice WRLD", "Goodbye & Good Riddance", "Lucid Dreams", 239_000);
        assert_eq!(uri, "spotify:local:Juice+WRLD:Goodbye+%26+Good+Riddance:Lucid+Dreams:239");
        let parsed = parse_uri(&uri).unwrap();
        assert_eq!(parsed.artist, "Juice WRLD");
        assert_eq!(parsed.album, "Goodbye & Good Riddance");
        assert_eq!(parsed.title, "Lucid Dreams");
        assert_eq!(parsed.seconds, 239);
        assert!(parse_uri("spotify:track:abc").is_none());
        assert!(parse_uri("spotify:local:a:b").is_none());
        assert_eq!(parse_uri("spotify:local:a%:b:c%2:5").unwrap().artist, "a%");
    }
}
