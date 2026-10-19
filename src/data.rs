//! Where game data lives, and Freedoom's download.
//!
//! No id Software data ships with this program. The IWAD is one the user
//! owns (`--iwad`), or Freedoom (BSD licence), downloaded on request into
//! the data directory: `$XDG_DATA_HOME/rs-rich-doom` (else
//! `~/.local/share/rs-rich-doom`), `~/Library/Application Support/rs-rich-doom`
//! on macOS, `%APPDATA%\rs-rich-doom` on Windows. `RS_RICH_DOOM_DATA`
//! overrides it.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The Freedoom release fetched.
pub const FREEDOOM_VERSION: &str = "0.13.0";
pub const FREEDOOM_URL: &str =
    "https://github.com/freedoom/freedoom/releases/download/v0.13.0/freedoom-0.13.0.zip";
/// The zip's SHA-256, checked before anything is unpacked.
pub const FREEDOOM_SHA256: &str =
    "3f9b264f3e3ce503b4fb7f6bdcb1f419d93c7b546f4df3e874dd878db9688f59";
/// The IWADs in it, in the order they are preferred.
pub const FREEDOOM_WADS: [&str; 2] = ["freedoom1.wad", "freedoom2.wad"];

/// The data directory for `env` (a variable lookup) on this platform.
pub fn data_dir_with(env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(dir) = env("RS_RICH_DOOM_DATA").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(windows) {
        PathBuf::from(env("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(env("HOME")?).join("Library/Application Support")
    } else {
        match env("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(env("HOME")?).join(".local/share"),
        }
    };
    Some(base.join("rs-rich-doom"))
}

/// The data directory of this process.
pub fn data_dir() -> Option<PathBuf> {
    data_dir_with(|name| std::env::var(name).ok())
}

/// The IWAD to play: `explicit` if given, else Freedoom in `dir`.
pub fn find_iwad(explicit: Option<&Path>, dir: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return if path.is_file() {
            Ok(path.to_path_buf())
        } else {
            Err(format!("no IWAD at {}", path.display()))
        };
    }
    if let Some(dir) = dir {
        for name in FREEDOOM_WADS {
            let path = dir.join(name);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    Err("no IWAD: pass --iwad PATH (doom1.wad, doom.wad, doom2.wad, …) \
         or run `rs-rich-doom fetch-freedoom` to download Freedoom"
        .into())
}

/// Hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Unpack Freedoom's IWADs (and its licence) from the release zip into
/// `dir`. Returns the files written.
pub fn unpack_freedoom(zip_bytes: &[u8], dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|e| format!("not a zip: {e}"))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut written = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = file
            .enclosed_name()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        else {
            continue;
        };
        let wanted = FREEDOOM_WADS.contains(&name.as_str()) || name == "COPYING.txt";
        if !wanted || file.is_dir() {
            continue;
        }
        let target = dir.join(if name == "COPYING.txt" {
            "FREEDOOM-COPYING.txt"
        } else {
            &name
        });
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let partial = target.with_extension("part");
        std::fs::write(&partial, &bytes).map_err(|e| format!("{}: {e}", partial.display()))?;
        std::fs::rename(&partial, &target).map_err(|e| format!("{}: {e}", target.display()))?;
        written.push(target);
    }
    if !written.iter().any(|p| p.extension().is_some_and(|e| e == "wad")) {
        return Err("the zip holds no Freedoom IWAD".into());
    }
    Ok(written)
}

/// Download Freedoom into `dir` (unless it is there and not `force`).
pub fn fetch_freedoom(dir: &Path, force: bool) -> Result<Vec<PathBuf>, String> {
    if !force {
        let present: Vec<PathBuf> = FREEDOOM_WADS
            .iter()
            .map(|n| dir.join(n))
            .filter(|p| p.is_file())
            .collect();
        if present.len() == FREEDOOM_WADS.len() {
            return Ok(present);
        }
    }
    eprintln!("Downloading Freedoom {FREEDOOM_VERSION} from {FREEDOOM_URL} …");
    let mut response = ureq::get(FREEDOOM_URL)
        .call()
        .map_err(|e| format!("download failed: {e}"))?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("download failed: {e}"))?;
    let digest = sha256_hex(&bytes);
    if digest != FREEDOOM_SHA256 {
        return Err(format!(
            "the download's SHA-256 is {digest}, not Freedoom {FREEDOOM_VERSION}'s {FREEDOOM_SHA256}"
        ));
    }
    unpack_freedoom(&bytes, dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn the_data_directory() {
        assert_eq!(
            data_dir_with(env(&[("RS_RICH_DOOM_DATA", "/x"), ("HOME", "/h")])),
            Some(PathBuf::from("/x"))
        );
        if cfg!(all(unix, not(target_os = "macos"))) {
            assert_eq!(
                data_dir_with(env(&[("HOME", "/h")])),
                Some(PathBuf::from("/h/.local/share/rs-rich-doom"))
            );
            assert_eq!(
                data_dir_with(env(&[("HOME", "/h"), ("XDG_DATA_HOME", "/d")])),
                Some(PathBuf::from("/d/rs-rich-doom"))
            );
        }
        assert_eq!(data_dir_with(env(&[])), None);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rs-rich-doom-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_the_iwad() {
        let dir = scratch("find");
        assert!(find_iwad(None, Some(&dir)).unwrap_err().contains("fetch-freedoom"));
        std::fs::write(dir.join("freedoom2.wad"), b"IWAD").unwrap();
        assert_eq!(find_iwad(None, Some(&dir)).unwrap(), dir.join("freedoom2.wad"));
        std::fs::write(dir.join("freedoom1.wad"), b"IWAD").unwrap();
        assert_eq!(find_iwad(None, Some(&dir)).unwrap(), dir.join("freedoom1.wad"));
        let mine = dir.join("doom1.wad");
        assert!(find_iwad(Some(&mine), Some(&dir)).is_err());
        std::fs::write(&mine, b"IWAD").unwrap();
        assert_eq!(find_iwad(Some(&mine), Some(&dir)).unwrap(), mine);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unpacks_only_the_iwads_and_licence() {
        let mut zip_bytes = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_bytes));
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (name, body) in [
                ("freedoom-0.13.0/freedoom1.wad", "IWAD1"),
                ("freedoom-0.13.0/COPYING.txt", "BSD"),
                ("freedoom-0.13.0/README.html", "no"),
                ("../evil.wad", "no"),
            ] {
                zip.start_file(name, options).unwrap();
                zip.write_all(body.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        let dir = scratch("unpack");
        let written = unpack_freedoom(&zip_bytes, &dir).unwrap();
        assert_eq!(written.len(), 2);
        assert_eq!(std::fs::read(dir.join("freedoom1.wad")).unwrap(), b"IWAD1");
        assert_eq!(std::fs::read(dir.join("FREEDOOM-COPYING.txt")).unwrap(), b"BSD");
        assert!(!dir.join("README.html").exists());
        assert!(unpack_freedoom(b"nope", &dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hashes() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
