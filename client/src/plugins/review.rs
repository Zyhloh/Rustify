use super::manifest::{Kind, Manifest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Note,
    Review,
    Caution,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Note => "note",
            Level::Review => "review",
            Level::Caution => "caution",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub endpoints: Vec<String>,
}

impl Report {
    pub fn level(&self) -> Option<Level> {
        self.findings.iter().map(|f| f.level).filter(|l| *l > Level::Note).max()
    }

    fn add(&mut self, level: Level, text: impl Into<String>) {
        self.findings.push(Finding { level, text: text.into() });
    }
}

fn calls(script: &str, name: &str) -> bool {
    let bytes = script.as_bytes();
    script.match_indices(name).any(|(i, _)| {
        let before = i.checked_sub(1).map(|j| bytes[j]);
        let ident = before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'.');
        let rest = script[i + name.len()..].trim_start();
        !ident && rest.starts_with('(')
    })
}

fn escapes(script: &str) -> usize {
    script.matches("\\x").count() + script.matches("\\u").count()
}

fn longest_blob(script: &str) -> usize {
    let mut best = 0;
    let mut run = 0;
    for b in script.bytes() {
        if b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best
}

fn endpoints(script: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, _) in script.match_indices("/v1/") {
        let path: String = script[i..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
            .collect();
        let path = path.trim_end_matches('/').to_owned();
        if path.len() > 4 && !out.contains(&path) {
            out.push(path);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn luminance(hex: &str) -> Option<f64> {
    let raw = hex.strip_prefix('#')?;
    let channel = |i: usize| -> Option<f64> {
        let v = u8::from_str_radix(raw.get(i..i + 2)?, 16).ok()? as f64 / 255.0;
        Some(if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) })
    };
    Some(0.2126 * channel(0)? + 0.7152 * channel(2)? + 0.0722 * channel(4)?)
}

pub fn contrast(a: &str, b: &str) -> Option<f64> {
    let (x, y) = (luminance(a)?, luminance(b)?);
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    Some((hi + 0.05) / (lo + 0.05))
}

pub fn review(m: &Manifest, script: &str) -> Report {
    let mut report = Report::default();
    let has = |p: &str| m.permissions.iter().any(|x| x == p);

    if m.kind == Kind::Plugin {
        if calls(script, "eval") || script.contains("new Function") || calls(script, "Function") {
            report.add(Level::Caution, "Builds and runs code from text while running, which hides what it really does.");
        }
        if script.contains("setTimeout(\"") || script.contains("setInterval(\"") || script.contains("setTimeout('") || script.contains("setInterval('") {
            report.add(Level::Caution, "Passes code as text to a timer, a common way to hide behavior.");
        }
        if escapes(script) > 150 {
            report.add(Level::Caution, "Contains a lot of encoded text, a common way to hide code.");
        } else if longest_blob(script) > 1500 {
            report.add(Level::Caution, "Contains a large encoded block of data.");
        }
        if calls(script, "atob") || script.matches("fromCharCode").count() > 3 {
            report.add(Level::Review, "Decodes hidden text while running.");
        }
        let lines = script.lines().count().max(1);
        if script.lines().any(|l| l.len() > 4000) || (script.len() > 20_000 && script.len() / lines > 500) {
            report.add(Level::Review, "The code is minified, so it's hard to check what it does. Prefer plugins with readable source.");
        }
        let compact: String = script.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.contains("while(true)") || compact.contains("while(1)") || compact.contains("for(;;)") {
            report.add(Level::Review, "Has a loop that never ends on its own, which can freeze it. Rustify stops plugins that hang.");
        }
        if !has("network") && (calls(script, "fetch") || script.contains("XMLHttpRequest") || script.contains("WebSocket")) {
            report.add(Level::Review, "Tries to connect to the internet without asking. Rustify blocks this, but ask the author why.");
        }
        if !m.network.is_empty() {
            report.add(Level::Note, format!("Connects to {}.", m.network.join(", ")));
        }
        if script.contains("document.") || script.contains("React.") || script.contains("ReactDOM") {
            report.add(Level::Note, "Uses browser page features that Rustify doesn't have. Those parts won't work.");
        }

        report.endpoints = if has("spotify") { endpoints(script) } else { Vec::new() };
        if has("spotify") {
            let reads_profile = report.endpoints.iter().any(|e| e == "/v1/me") || script.contains("\"/v1/me\"") || script.contains("'/v1/me'");
            if reads_profile {
                report.add(Level::Review, "Reads your Spotify profile, which includes your email address and country.");
            }
        }
        if has("player")
            && (script.contains("setInterval") || script.contains("setTimeout"))
            && [".next(", ".pause(", ".setVolume(", ".seek(", ".back("].iter().any(|c| script.contains(c))
        {
            report.add(Level::Review, "Can change playback on its own, without you clicking anything.");
        }
        if PERMISSIONS_ALL.iter().all(|p| has(p)) {
            report.add(Level::Review, "Asks for every permission.");
        }
    }

    if let (Some(text), Some(base)) = (m.theme.get("text"), m.theme.get("base"))
        && contrast(text, base).is_some_and(|c| c < 4.5)
    {
        report.add(Level::Review, "Text may be hard to read with these colors.");
    }

    if m.repository.is_empty() && m.homepage.is_empty() {
        report.add(Level::Note, "There's no link to its source, so you can't check where it came from.");
    }
    report
}

const PERMISSIONS_ALL: [&str; 3] = ["player", "spotify", "theme"];

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(perms: &[&str]) -> Manifest {
        Manifest {
            id: "x-plugin".into(),
            name: "X".into(),
            main: "index.js".into(),
            repository: "https://github.com/a/b".into(),
            permissions: perms.iter().map(|p| p.to_string()).collect(),
            ..Manifest::default()
        }
    }

    #[test]
    fn clean_plugin_has_no_warnings() {
        let r = review(&plugin(&["player"]), "Rustify.UI.Playbar.Button('Skip', 'next', () => Rustify.Player.next());");
        assert_eq!(r.level(), None);
    }

    #[test]
    fn flags_eval_and_obfuscation() {
        let r = review(&plugin(&[]), "eval(atob('ZG9jdW1lbnQ='))");
        assert_eq!(r.level(), Some(Level::Caution));
        let r = review(&plugin(&[]), "const evaluate = 1; x.eval(2); retrieval(3)");
        assert_eq!(r.level(), None);
        let blob = format!("const a = \"{}\";", "QUJD".repeat(500));
        assert_eq!(review(&plugin(&[]), &blob).level(), Some(Level::Caution));
        let encoded = "\\x41".repeat(200);
        assert_eq!(review(&plugin(&[]), &encoded).level(), Some(Level::Caution));
        assert_eq!(review(&plugin(&[]), "new Function('return 1')()").level(), Some(Level::Caution));
    }

    #[test]
    fn flags_profile_reads_and_lists_endpoints() {
        let r = review(&plugin(&["spotify"]), "Rustify.Spotify.get('/v1/me'); Rustify.Spotify.get('/v1/me/top/tracks?limit=5')");
        assert_eq!(r.level(), Some(Level::Review));
        assert_eq!(r.endpoints, vec!["/v1/me", "/v1/me/top/tracks"]);
    }

    #[test]
    fn flags_automatic_playback_changes() {
        let r = review(&plugin(&["player"]), "setInterval(() => Rustify.Player.next(), 1000)");
        assert_eq!(r.level(), Some(Level::Review));
    }

    #[test]
    fn flags_endless_loops() {
        assert_eq!(review(&plugin(&[]), "Rustify.on('x', () => { while ( true ) {} })").level(), Some(Level::Review));
        assert_eq!(review(&plugin(&[]), "for (;;) {}").level(), Some(Level::Review));
        assert_eq!(review(&plugin(&[]), "while (i < 3) i++").level(), None);
    }

    #[test]
    fn flags_network_attempts() {
        assert_eq!(review(&plugin(&[]), "fetch('https://evil.example')").level(), Some(Level::Review));
    }

    #[test]
    fn flags_unreadable_themes() {
        let mut m = plugin(&[]);
        m.kind = Kind::Theme;
        m.theme.insert("text".into(), "#222222".into());
        m.theme.insert("base".into(), "#111111".into());
        assert_eq!(review(&m, "").level(), Some(Level::Review));
    }

    #[test]
    fn missing_source_is_only_a_note() {
        let mut m = plugin(&[]);
        m.repository.clear();
        let r = review(&m, "");
        assert_eq!(r.level(), None);
        assert_eq!(r.findings.len(), 1);
    }
}
