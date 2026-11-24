use serde::Deserialize;
use std::{collections::BTreeMap, path::Path};

pub const PERMISSIONS: [&str; 3] = ["player", "spotify", "theme"];
const MAX_SCRIPT: u64 = 512 * 1024;
pub const MAX_ICON: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Plugin,
    Theme,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Plugin => "plugin",
            Kind::Theme => "theme",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Manifest {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub version: String,
    pub author: String,
    pub author_url: String,
    pub description: String,
    pub repository: String,
    pub homepage: String,
    pub icon: String,
    pub main: String,
    pub permissions: Vec<String>,
    pub theme: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Author {
    Name(String),
    Full {
        name: String,
        #[serde(default)]
        url: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    id: String,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    author: Option<Author>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    repository: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    main: Option<String>,
    #[serde(default)]
    permissions: Vec<String>,
    #[serde(default)]
    theme: BTreeMap<String, String>,
}

pub fn valid_id(id: &str) -> bool {
    (3..=40).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

fn valid_name(name: &str, extensions: &[&str]) -> bool {
    (1..=64).contains(&name.len())
        && extensions.iter().any(|e| name.to_ascii_lowercase().ends_with(e))
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !name.starts_with('.')
}

fn valid_file(name: &str) -> bool {
    valid_name(name, &[".js"])
}

pub fn valid_icon_file(name: &str) -> bool {
    valid_name(name, &[".png", ".jpg", ".jpeg"])
}

pub fn valid_link(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    url.len() <= 200
        && host.contains('.')
        && !host.contains('@')
        && !host.contains(':')
        && url.chars().all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]!$&()*+,;=%".contains(c))
}

pub fn link_label(url: &str) -> String {
    let rest = url.trim_start_matches("https://").trim_start_matches("www.");
    let trimmed = rest.trim_end_matches('/');
    if trimmed.len() > 48 { format!("{}…", &trimmed[..47]) } else { trimmed.to_owned() }
}

fn clean(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_owned()
}

pub fn valid_color(value: &str) -> bool {
    let hex = value.strip_prefix('#').unwrap_or("");
    matches!(hex.len(), 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

pub const THEME_KEYS: [&str; 15] = [
    "base", "surface", "surface-raised", "surface-high", "tile", "tile-hover", "hover", "press", "text", "text-dim",
    "text-faint", "accent", "accent-hover", "danger", "track",
];

pub fn check_theme(theme: &BTreeMap<String, String>) -> Result<(), String> {
    for (key, value) in theme {
        if !THEME_KEYS.contains(&key.as_str()) {
            return Err(format!("unknown theme color \"{key}\""));
        }
        if !valid_color(value) {
            return Err(format!("theme color \"{key}\" must be #rrggbb or #rrggbbaa"));
        }
    }
    Ok(())
}

pub fn parse(raw: &[u8], folder: &str) -> Result<Manifest, String> {
    if raw.len() > 64 * 1024 {
        return Err("manifest.json is too large".into());
    }
    let body = raw.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(raw);
    let raw: Raw = serde_json::from_slice(body).map_err(|e| format!("manifest.json: {e}"))?;
    if !valid_id(&raw.id) {
        return Err("id must be 3-40 characters of a-z, 0-9 and -".into());
    }
    if raw.id != folder {
        return Err(format!("folder must be named \"{}\"", raw.id));
    }
    let kind = match raw.kind.as_deref() {
        None | Some("plugin") => Kind::Plugin,
        Some("theme") => Kind::Theme,
        Some(other) => return Err(format!("unknown type \"{other}\", use \"plugin\" or \"theme\"")),
    };
    if let Some(bad) = raw.permissions.iter().find(|p| !PERMISSIONS.contains(&p.as_str())) {
        return Err(format!("unknown permission \"{bad}\""));
    }
    check_theme(&raw.theme)?;
    let mut permissions = raw.permissions;
    let main = match kind {
        Kind::Theme => {
            if raw.theme.is_empty() {
                return Err("a theme needs at least one color in \"theme\"".into());
            }
            if raw.main.is_some() || permissions.iter().any(|p| p != "theme") {
                return Err("themes only contain colors, they can't run code or ask for permissions".into());
            }
            permissions = vec!["theme".into()];
            String::new()
        }
        Kind::Plugin => {
            if !raw.theme.is_empty() && !permissions.iter().any(|p| p == "theme") {
                return Err("colors in \"theme\" need the \"theme\" permission".into());
            }
            let main = raw.main.unwrap_or_else(|| "index.js".into());
            if !valid_file(&main) {
                return Err("main must be a .js file inside the plugin folder".into());
            }
            main
        }
    };
    permissions.sort();
    permissions.dedup();
    let (author, author_url) = match raw.author {
        None => (String::new(), String::new()),
        Some(Author::Name(name)) => (name, String::new()),
        Some(Author::Full { name, url }) => (name, url),
    };
    for (field, url) in [("author url", &author_url), ("repository", &raw.repository), ("homepage", &raw.homepage)] {
        if !url.is_empty() && !valid_link(url) {
            return Err(format!("{field} must be an https:// link"));
        }
    }
    if !raw.icon.is_empty() && !valid_icon_file(&raw.icon) {
        return Err("icon must be a .png or .jpg file inside the folder".into());
    }
    let manifest = Manifest {
        id: raw.id,
        kind,
        name: clean(&raw.name, 40),
        version: clean(&raw.version, 20),
        author: clean(&author, 40),
        author_url,
        description: clean(&raw.description, 300),
        repository: raw.repository,
        homepage: raw.homepage,
        icon: raw.icon,
        main,
        permissions,
        theme: raw.theme,
    };
    if manifest.name.is_empty() {
        return Err("name is required".into());
    }
    Ok(manifest)
}

pub fn load(dir: &Path) -> Result<(Manifest, String), String> {
    let folder = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
    let raw = std::fs::read(dir.join("manifest.json")).map_err(|_| "missing manifest.json".to_string())?;
    let mut manifest = parse(&raw, &folder)?;
    if !manifest.icon.is_empty() {
        let ok = std::fs::metadata(dir.join(&manifest.icon)).is_ok_and(|m| m.is_file() && m.len() <= MAX_ICON);
        if !ok {
            manifest.icon.clear();
        }
    }
    if manifest.kind == Kind::Theme {
        return Ok((manifest, String::new()));
    }
    let script_path = dir.join(&manifest.main);
    let size = std::fs::metadata(&script_path).map_err(|_| format!("missing {}", manifest.main))?.len();
    if size > MAX_SCRIPT {
        return Err(format!("{} is larger than 512 KB", manifest.main));
    }
    let script = std::fs::read_to_string(&script_path).map_err(|_| format!("{} is not valid UTF-8", manifest.main))?;
    Ok((manifest, script.trim_start_matches('\u{feff}').to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, manifest: &str, script: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        std::fs::write(dir.join("index.js"), script).unwrap();
    }

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rustify-manifest-{}-{name}", std::process::id())).join(name)
    }

    #[test]
    fn ids() {
        assert!(valid_id("now-playing"));
        assert!(!valid_id("ab"));
        assert!(!valid_id("../evil"));
        assert!(!valid_id("Upper"));
        assert!(!valid_id("-dash"));
    }

    #[test]
    fn files() {
        assert!(valid_file("index.js"));
        assert!(!valid_file("../index.js"));
        assert!(!valid_file("sub/index.js"));
        assert!(!valid_file("index.ts"));
        assert!(!valid_file(".hidden.js"));
        assert!(valid_icon_file("icon.png"));
        assert!(!valid_icon_file("icon.svg"));
        assert!(!valid_icon_file("../icon.png"));
    }

    #[test]
    fn links() {
        assert!(valid_link("https://github.com/someone/plugin"));
        assert!(!valid_link("http://github.com/someone/plugin"));
        assert!(!valid_link("javascript:alert(1)"));
        assert!(!valid_link("https://user@evil.com/"));
        assert!(!valid_link("https://github.com/a\"b"));
        assert!(!valid_link("https://localhost/"));
        assert_eq!(link_label("https://www.github.com/someone/plugin/"), "github.com/someone/plugin");
    }

    #[test]
    fn colors() {
        assert!(valid_color("#1ed760"));
        assert!(valid_color("#1ed760cc"));
        assert!(!valid_color("red"));
        assert!(!valid_color("#12345"));
        assert!(!valid_color("#zzzzzz"));
    }

    #[test]
    fn loads_valid_plugin() {
        let dir = temp("good-plugin");
        write(
            &dir,
            r#"{"id":"good-plugin","name":"Good","version":"1.0.0","permissions":["player","player"],"author":{"name":"Someone","url":"https://github.com/someone"},"repository":"https://github.com/someone/good-plugin"}"#,
            "1",
        );
        let (m, script) = load(&dir).unwrap();
        assert_eq!(m.permissions, vec!["player"]);
        assert_eq!(m.author, "Someone");
        assert_eq!(m.author_url, "https://github.com/someone");
        assert_eq!(m.kind, Kind::Plugin);
        assert_eq!(script, "1");
    }

    #[test]
    fn loads_theme_without_script() {
        let dir = temp("dark-ocean");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), r##"{"id":"dark-ocean","type":"theme","name":"Ocean","author":"x","theme":{"accent":"#00aaff"}}"##).unwrap();
        let (m, script) = load(&dir).unwrap();
        assert_eq!(m.kind, Kind::Theme);
        assert_eq!(m.permissions, vec!["theme"]);
        assert!(script.is_empty());
    }

    #[test]
    fn rejects_bad_plugins() {
        let dir = temp("wrong-folder");
        write(&dir, r#"{"id":"other-name","name":"X","version":"1"}"#, "");
        assert!(load(&dir).unwrap_err().contains("folder"));

        let dir = temp("escape");
        write(&dir, r#"{"id":"escape","name":"X","version":"1","main":"../../x.js"}"#, "");
        assert!(load(&dir).is_err());

        let dir = temp("perm");
        write(&dir, r#"{"id":"perm","name":"X","version":"1","permissions":["filesystem"]}"#, "");
        assert!(load(&dir).unwrap_err().contains("permission"));

        let dir = temp("themed");
        write(&dir, r##"{"id":"themed","name":"X","version":"1","theme":{"accent":"#ff0000"}}"##, "");
        assert!(load(&dir).unwrap_err().contains("theme"));

        let dir = temp("extra");
        write(&dir, r#"{"id":"extra","name":"X","version":"1","exec":"calc"}"#, "");
        assert!(load(&dir).is_err());

        let dir = temp("theme-code");
        write(&dir, r##"{"id":"theme-code","type":"theme","name":"X","main":"index.js","theme":{"accent":"#ff0000"}}"##, "");
        assert!(load(&dir).is_err());

        let dir = temp("bad-link");
        write(&dir, r#"{"id":"bad-link","name":"X","repository":"javascript:alert(1)"}"#, "");
        assert!(load(&dir).unwrap_err().contains("https"));

        let dir = temp("bad-type");
        write(&dir, r#"{"id":"bad-type","type":"native","name":"X"}"#, "");
        assert!(load(&dir).is_err());
    }
}
