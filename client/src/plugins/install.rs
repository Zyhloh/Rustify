use super::manifest::{self, Kind};
use std::{
    io::{Cursor, Read},
    path::{Component, Path},
};

pub const MAX_ARCHIVE: usize = 5 * 1024 * 1024;
const MAX_TOTAL: u64 = 8 * 1024 * 1024;
const MAX_FILES: usize = 300;
const MAX_DEPTH: usize = 6;
const ALLOWED: [&str; 7] = ["json", "js", "png", "jpg", "jpeg", "md", "txt"];
const BLOCKED: [&str; 19] = [
    "exe", "dll", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "scr", "msi", "com", "lnk", "jar", "sys", "hta", "wsf", "reg", "cpl", "url",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub replaced: bool,
}

struct Entry {
    path: Vec<String>,
    data: Vec<u8>,
}

fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

fn parts(path: &Path) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => out.push(p.to_str()?.to_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.is_empty() && out.len() <= MAX_DEPTH).then_some(out)
}

fn read_zip(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    if bytes.len() > MAX_ARCHIVE {
        return Err("the file is larger than 5 MB".into());
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| "this isn't a valid .zip file".to_string())?;
    if archive.len() > MAX_FILES * 4 {
        return Err("the archive has too many files".into());
    }
    let mut total = 0u64;
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| "the archive is damaged".to_string())?;
        if file.is_dir() {
            continue;
        }
        let Some(path) = file.enclosed_name().and_then(|p| parts(&p)) else {
            return Err("the archive contains an unsafe file path".into());
        };
        let name = path.last().cloned().unwrap_or_default();
        if BLOCKED.contains(&extension(&name).as_str()) {
            return Err(format!("it contains a program file ({name}), which plugins never need"));
        }
        if !ALLOWED.contains(&extension(&name).as_str()) && !name.eq_ignore_ascii_case("license") {
            continue;
        }
        if entries.len() >= MAX_FILES {
            return Err("the archive has too many files".into());
        }
        let remaining = MAX_TOTAL - total;
        if file.size() > remaining {
            return Err("the unpacked files are larger than 8 MB".into());
        }
        let mut data = Vec::with_capacity(file.size() as usize);
        (&mut file).take(remaining + 1).read_to_end(&mut data).map_err(|_| "the archive is damaged".to_string())?;
        total += data.len() as u64;
        if total > MAX_TOTAL {
            return Err("the unpacked files are larger than 8 MB".into());
        }
        entries.push(Entry { path, data });
    }
    Ok(entries)
}

fn read_folder(dir: &Path) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![(dir.to_path_buf(), Vec::<String>::new())];
    while let Some((path, prefix)) = stack.pop() {
        let list = std::fs::read_dir(&path).map_err(|_| "couldn't read that folder".to_string())?;
        for item in list.flatten() {
            let meta = std::fs::symlink_metadata(item.path()).map_err(|_| "couldn't read that folder".to_string())?;
            let Some(name) = item.file_name().to_str().map(str::to_owned) else { continue };
            let mut rel = prefix.clone();
            rel.push(name.clone());
            if meta.file_type().is_symlink() || rel.len() > MAX_DEPTH || name.starts_with('.') {
                continue;
            }
            if meta.is_dir() {
                stack.push((item.path(), rel));
                continue;
            }
            if BLOCKED.contains(&extension(&name).as_str()) {
                return Err(format!("it contains a program file ({name}), which plugins never need"));
            }
            if !ALLOWED.contains(&extension(&name).as_str()) && !name.eq_ignore_ascii_case("license") {
                continue;
            }
            total += meta.len();
            if entries.len() >= MAX_FILES || total > MAX_TOTAL {
                return Err("the folder is too large".into());
            }
            let data = std::fs::read(item.path()).map_err(|_| "couldn't read that folder".to_string())?;
            entries.push(Entry { path: rel, data });
        }
    }
    Ok(entries)
}

fn install(mut entries: Vec<Entry>, sub: &[String], root: &Path, staging: &Path) -> Result<Installed, String> {
    let top = entries.first().map(|e| e.path[0].clone());
    if let Some(top) = top
        && entries.iter().all(|e| e.path.len() > 1 && e.path[0] == top)
    {
        for e in &mut entries {
            e.path.remove(0);
        }
    }
    let manifest = entries
        .iter()
        .filter(|e| e.path.last().is_some_and(|n| n == "manifest.json") && e.path.starts_with(sub))
        .min_by_key(|e| e.path.len())
        .ok_or_else(|| "there's no manifest.json in it".to_string())?;
    let base = manifest.path[..manifest.path.len() - 1].to_vec();
    let value: serde_json::Value = serde_json::from_slice(manifest.data.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&manifest.data))
        .map_err(|e| format!("manifest.json: {e}"))?;
    let id = value.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
    if !manifest::valid_id(&id) {
        return Err("manifest.json has an invalid id".into());
    }
    manifest::parse(&manifest.data, &id)?;

    let _ = std::fs::remove_dir_all(staging);
    let stage = staging.join(&id);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    for entry in entries.iter().filter(|e| e.path.len() > base.len() && e.path.starts_with(&base)) {
        let rel = &entry.path[base.len()..];
        let mut target = stage.clone();
        for part in rel {
            target.push(part);
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&target, &entry.data).map_err(|e| e.to_string())?;
    }
    let (m, _) = match manifest::load(&stage) {
        Ok(found) => found,
        Err(e) => {
            let _ = std::fs::remove_dir_all(staging);
            return Err(e);
        }
    };
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let target = root.join(&id);
    let replaced = target.exists();
    if replaced {
        std::fs::remove_dir_all(&target).map_err(|_| "couldn't replace the installed copy, is a file open?".to_string())?;
    }
    std::fs::rename(&stage, &target).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(staging);
    Ok(Installed { id, name: m.name, kind: m.kind, replaced })
}

pub fn from_zip(bytes: &[u8], sub: &[String], root: &Path, staging: &Path) -> Result<Installed, String> {
    install(read_zip(bytes)?, sub, root, staging)
}

pub fn from_path(path: &Path, root: &Path, staging: &Path) -> Result<Installed, String> {
    let meta = std::fs::metadata(path).map_err(|_| "couldn't open that file".to_string())?;
    if meta.is_dir() {
        if path.starts_with(root) {
            return Err("that folder is already in the plugins folder".into());
        }
        let entries = read_folder(path)?;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let wrapped = entries.into_iter().map(|e| Entry { path: std::iter::once(name.clone()).chain(e.path).collect(), data: e.data }).collect();
        return install(wrapped, &[], root, staging);
    }
    if !path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        return Err("drop a .zip file or a plugin folder".into());
    }
    if meta.len() as usize > MAX_ARCHIVE {
        return Err("the file is larger than 5 MB".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "couldn't open that file".to_string())?;
    from_zip(&bytes, &[], root, staging)
}

#[derive(Debug, Clone, PartialEq)]
pub struct GithubSource {
    pub zip_url: String,
    pub sub: Vec<String>,
}

fn github_name(part: &str) -> bool {
    (1..=100).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')) && part != ".." && part != "."
}

pub fn github_source(url: &str) -> Option<GithubSource> {
    let url = url.trim();
    let rest = url.strip_prefix("https://github.com/").or_else(|| url.strip_prefix("github.com/"))?;
    let rest = rest.split(['?', '#']).next()?.trim_end_matches('/');
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?.trim_end_matches(".git");
    if !github_name(owner) || !github_name(repo) {
        return None;
    }
    let (reference, sub) = match parts.next() {
        None => ("HEAD".to_owned(), Vec::new()),
        Some("tree") => {
            let branch = parts.next()?;
            if !github_name(branch) {
                return None;
            }
            let sub: Vec<String> = parts.map(str::to_owned).collect();
            if sub.len() > MAX_DEPTH || !sub.iter().all(|p| github_name(p)) {
                return None;
            }
            (format!("refs/heads/{branch}"), sub)
        }
        Some(_) => return None,
    };
    Some(GithubSource { zip_url: format!("https://codeload.github.com/{owner}/{repo}/zip/{reference}"), sub })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, data) in files {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        out.into_inner()
    }

    fn dirs(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("rustify-install-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        (base.join("plugins"), base.join("staging"))
    }

    const MANIFEST: &[u8] = br#"{"id":"cool-plugin","name":"Cool","version":"1.0.0"}"#;

    #[test]
    fn installs_github_style_archive() {
        let (root, staging) = dirs("github");
        let bytes = zip(&[("cool-plugin-main/manifest.json", MANIFEST), ("cool-plugin-main/index.js", b"1"), ("cool-plugin-main/README.md", b"hi")]);
        let installed = from_zip(&bytes, &[], &root, &staging).unwrap();
        assert_eq!(installed.id, "cool-plugin");
        assert!(!installed.replaced);
        assert!(root.join("cool-plugin/index.js").exists());
        assert!(!staging.exists());
        let again = from_zip(&bytes, &[], &root, &staging).unwrap();
        assert!(again.replaced);
    }

    #[test]
    fn installs_from_subfolder() {
        let (root, staging) = dirs("sub");
        let bytes = zip(&[
            ("repo-main/plugins/cool-plugin/manifest.json", MANIFEST),
            ("repo-main/plugins/cool-plugin/index.js", b"1"),
            ("repo-main/other/manifest.json", b"{}"),
        ]);
        let sub = vec!["plugins".to_owned(), "cool-plugin".to_owned()];
        assert_eq!(from_zip(&bytes, &sub, &root, &staging).unwrap().id, "cool-plugin");
    }

    #[test]
    fn rejects_dangerous_archives() {
        let (root, staging) = dirs("bad");
        let slip = zip(&[("../../evil.js", b"x"), ("manifest.json", MANIFEST)]);
        assert!(from_zip(&slip, &[], &root, &staging).unwrap_err().contains("unsafe"));
        let exe = zip(&[("manifest.json", MANIFEST), ("index.js", b"1"), ("helper.exe", b"MZ")]);
        assert!(from_zip(&exe, &[], &root, &staging).unwrap_err().contains("program"));
        let bomb = zip(&[("manifest.json", MANIFEST), ("index.js", &vec![b' '; 9 * 1024 * 1024])]);
        assert!(from_zip(&bomb, &[], &root, &staging).unwrap_err().contains("8 MB"));
        assert!(from_zip(b"not a zip", &[], &root, &staging).is_err());
        let none = zip(&[("index.js", b"1")]);
        assert!(from_zip(&none, &[], &root, &staging).unwrap_err().contains("manifest"));
        let bad = zip(&[("manifest.json", br#"{"id":"../x","name":"x"}"#)]);
        assert!(from_zip(&bad, &[], &root, &staging).is_err());
        assert!(!root.join("..").join("evil.js").exists());
    }

    #[test]
    fn broken_plugin_leaves_nothing_behind() {
        let (root, staging) = dirs("broken");
        let bytes = zip(&[("manifest.json", MANIFEST)]);
        assert!(from_zip(&bytes, &[], &root, &staging).unwrap_err().contains("index.js"));
        assert!(!root.join("cool-plugin").exists());
    }

    #[test]
    fn installs_folder() {
        let (root, staging) = dirs("folder");
        let src = root.parent().unwrap().join("src").join("cool-plugin");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("manifest.json"), MANIFEST).unwrap();
        std::fs::write(src.join("index.js"), "1").unwrap();
        assert_eq!(from_path(&src, &root, &staging).unwrap().id, "cool-plugin");
    }

    #[test]
    fn parses_github_links() {
        assert_eq!(
            github_source("https://github.com/someone/cool-plugin"),
            Some(GithubSource { zip_url: "https://codeload.github.com/someone/cool-plugin/zip/HEAD".into(), sub: vec![] })
        );
        assert_eq!(
            github_source("https://github.com/someone/repo/tree/main/plugins/cool"),
            Some(GithubSource {
                zip_url: "https://codeload.github.com/someone/repo/zip/refs/heads/main".into(),
                sub: vec!["plugins".into(), "cool".into()]
            })
        );
        assert!(github_source("https://evil.com/someone/repo").is_none());
        assert!(github_source("https://github.com/someone/../x").is_none());
        assert!(github_source("https://github.com/someone/repo/blob/main/x.js").is_none());
        assert!(github_source("https://github.com/so me/repo").is_none());
    }
}
