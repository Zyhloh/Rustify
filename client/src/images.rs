use crate::config::Config;
use image::{GenericImageView, imageops::FilterType};
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::{
    collections::HashSet,
    hash::{DefaultHasher, Hash, Hasher},
    path::PathBuf,
    sync::Mutex,
};
use tokio::sync::Semaphore;

pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

pub struct Decoded {
    pub pixels: Pixels,
    pub accent: [f32; 3],
}

pub struct Images {
    http: reqwest::Client,
    dir: PathBuf,
    permits: Semaphore,
    pending: Mutex<HashSet<String>>,
}

pub fn key(url: &str, px: u32) -> String {
    format!("{url}#{px}")
}

impl Images {
    pub fn new() -> Self {
        let dir = Config::data_dir().join("images");
        let _ = std::fs::create_dir_all(&dir);
        Self {
            http: reqwest::Client::new(),
            dir,
            permits: Semaphore::new(8),
            pending: Mutex::new(HashSet::new()),
        }
    }

    pub async fn fetch(&self, key: &str) -> Option<Decoded> {
        if !self.pending.lock().ok()?.insert(key.to_owned()) {
            return None;
        }
        let result = self.load(key).await;
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(key);
        }
        result
    }

    async fn load(&self, key: &str) -> Option<Decoded> {
        let (url, px) = key.rsplit_once('#')?;
        let px: u32 = px.parse().ok()?;
        let _permit = self.permits.acquire().await.ok()?;

        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);
        let path = self.dir.join(format!("{:016x}", hasher.finish()));

        let bytes = match tokio::fs::read(&path).await {
            Ok(bytes) => bytes,
            Err(_) => {
                if !trusted(url) {
                    return None;
                }
                let response = self.http.get(url).send().await.ok()?.error_for_status().ok()?;
                if response.content_length().is_some_and(|len| len > MAX_BYTES) {
                    return None;
                }
                let bytes = response.bytes().await.ok()?.to_vec();
                if bytes.len() as u64 > MAX_BYTES {
                    return None;
                }
                let _ = tokio::fs::write(&path, &bytes).await;
                bytes
            }
        };

        tokio::task::spawn_blocking(move || decode(&bytes, px)).await.ok()?
    }
}

const MAX_BYTES: u64 = 8 * 1024 * 1024;

pub fn trusted(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return false };
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() || parsed.port().is_some() {
        return false;
    }
    let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) else { return false };
    ["scdn.co", "spotifycdn.com", "spotify.com", "fbcdn.net", "fbsbx.com"]
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

fn accent(img: &image::DynamicImage) -> [f32; 3] {
    let small = img.resize_exact(16, 16, FilterType::Triangle).to_rgb8();
    let (mut sum, mut weight) = ([0f32; 3], 0f32);
    for p in small.pixels() {
        let c = p.0.map(|v| v as f32 / 255.0);
        let w = c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2]) + 0.04;
        for i in 0..3 {
            sum[i] += c[i] * w;
        }
        weight += w;
    }
    sum.map(|v| v / weight.max(f32::EPSILON))
}

fn decode(bytes: &[u8], px: u32) -> Option<Decoded> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let (w, h) = img.dimensions();
    let short = w.min(h).max(1);
    let img = if short > px {
        let scale = px as f32 / short as f32;
        img.resize_exact(
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
            FilterType::Triangle,
        )
    } else {
        img
    };
    let accent = accent(&img);
    let rgba = img.to_rgba8();
    Some(Decoded {
        pixels: SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()),
        accent,
    })
}

#[cfg(test)]
mod tests {
    use super::trusted;

    #[test]
    fn image_hosts() {
        assert!(trusted("https://i.scdn.co/image/abc"));
        assert!(trusted("https://mosaic.scdn.co/640/abc"));
        assert!(!trusted("http://i.scdn.co/image/abc"));
        assert!(!trusted(r"https://evil.com\@i.scdn.co/x"));
        assert!(!trusted("https://user@i.scdn.co/x"));
        assert!(!trusted("https://i.scdn.co.evil.com/x"));
        assert!(!trusted("https://lh3.googleusercontent.com/x"));
        assert!(!trusted("file:///C:/Windows/win.ini"));
    }
}
